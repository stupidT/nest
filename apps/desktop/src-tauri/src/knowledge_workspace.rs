use crate::db::{self, NewChatFileChange};
use crate::error::{AppError, AppResult};
use crate::state::SharedState;
use crate::vault;
use std::collections::BTreeMap;
use std::path::Path;

pub const MAX_CHANGED_FILES: usize = 32;
pub const MAX_FILE_BYTES: usize = 256 * 1024;
pub const MAX_STAGED_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_LISTED_FILES: usize = 500;

#[allow(dead_code)]
pub const CAPABILITY_SEARCH: &str = "knowledge.search";
pub const CAPABILITY_LIST: &str = "knowledge.list";
pub const CAPABILITY_READ: &str = "knowledge.read";
pub const CAPABILITY_CREATE: &str = "knowledge.create";
pub const CAPABILITY_REPLACE: &str = "knowledge.replace";
pub const CAPABILITY_DELETE: &str = "knowledge.delete";

#[allow(dead_code)]
pub const ERR_INVALID_INPUT: &str = "invalid_input";
pub const ERR_NOT_FOUND: &str = "not_found";
pub const ERR_ALREADY_EXISTS: &str = "already_exists";
pub const ERR_PERMISSION_DENIED: &str = "permission_denied";
pub const ERR_PROTECTED_PATH: &str = "protected_path";
pub const ERR_REVIEW_LOCKED: &str = "review_locked";
#[allow(dead_code)]
pub const ERR_CONFLICT: &str = "conflict";
pub const ERR_LIMIT_EXCEEDED: &str = "limit_exceeded";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityMode {
    Ask,
    Agent,
}

#[derive(Debug, Clone)]
pub struct Capability {
    pub id: &'static str,
    #[allow(dead_code)]
    pub mode: CapabilityMode,
}

pub fn catalog_for_mode(mode: CapabilityMode) -> Vec<Capability> {
    let mut capabilities = vec![
        Capability {
            id: CAPABILITY_SEARCH,
            mode: CapabilityMode::Ask,
        },
        Capability {
            id: CAPABILITY_LIST,
            mode: CapabilityMode::Ask,
        },
        Capability {
            id: CAPABILITY_READ,
            mode: CapabilityMode::Ask,
        },
    ];
    if mode == CapabilityMode::Agent {
        capabilities.extend([
            Capability {
                id: CAPABILITY_CREATE,
                mode: CapabilityMode::Agent,
            },
            Capability {
                id: CAPABILITY_REPLACE,
                mode: CapabilityMode::Agent,
            },
            Capability {
                id: CAPABILITY_DELETE,
                mode: CapabilityMode::Agent,
            },
        ]);
    }
    capabilities
}

#[allow(dead_code)]
pub fn capability_allowed(mode: CapabilityMode, capability_id: &str) -> bool {
    catalog_for_mode(mode)
        .iter()
        .any(|capability| capability.id == capability_id)
}

#[derive(Debug, Clone)]
pub struct KnowledgeError {
    pub code: &'static str,
    pub message: String,
}

impl KnowledgeError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    #[allow(dead_code)]
    pub fn to_app_error(&self) -> AppError {
        AppError::msg(format!("{}: {}", self.code, self.message))
    }
}

impl std::fmt::Display for KnowledgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

pub type KnowledgeResult<T> = Result<T, KnowledgeError>;

fn invalid(message: impl Into<String>) -> KnowledgeError {
    KnowledgeError::new(ERR_INVALID_INPUT, message)
}

fn limit(message: impl Into<String>) -> KnowledgeError {
    KnowledgeError::new(ERR_LIMIT_EXCEEDED, message)
}

#[derive(Debug, Clone)]
struct StagedFile {
    original: Option<String>,
    current: Option<String>,
}

pub struct KnowledgeWorkspace {
    state: SharedState,
    mode: CapabilityMode,
    protected_paths: std::collections::HashSet<String>,
    staged: BTreeMap<String, StagedFile>,
}

#[allow(dead_code)]
pub struct KnowledgeHit {
    pub file_path: String,
    pub title: String,
    pub snippet: String,
    pub score: f32,
}

#[allow(dead_code)]
pub struct KnowledgeReadResult {
    pub path: String,
    pub content: String,
    pub truncated: bool,
}

pub struct KnowledgeListResult {
    pub files: Vec<String>,
    pub truncated: bool,
}

enum StageRequirement {
    Existing,
    Missing,
}

impl KnowledgeWorkspace {
    pub async fn search_turn(
        state: &SharedState,
        query: &str,
        limit: Option<u32>,
    ) -> KnowledgeResult<Vec<KnowledgeHit>> {
        let limit = limit.unwrap_or(5).clamp(1, 20);
        if query.trim().is_empty() {
            return Err(invalid("query must not be empty"));
        }
        let prefixes = {
            let conn = state.db.lock();
            db::list_sync_state(&conn)
                .map_err(|error| KnowledgeError::new("internal", error.to_string()))?
                .into_iter()
                .filter(|pack| pack.active)
                .map(|pack| pack.local_path)
                .collect::<Vec<_>>()
        };
        let citations =
            crate::retrieval::retrieve(&state.app_data_dir, state, query, &prefixes, limit)
                .await
                .map_err(|error| {
                    KnowledgeError::new("internal", format!("retrieval failed: {error}"))
                })?;
        Ok(citations
            .into_iter()
            .map(|citation| KnowledgeHit {
                file_path: citation.file_path,
                title: citation.title,
                snippet: citation.snippet,
                score: citation.score,
            })
            .collect())
    }

    pub fn open_turn(
        state: SharedState,
        mode: CapabilityMode,
        protected_paths: Vec<String>,
    ) -> Self {
        Self {
            state,
            mode,
            protected_paths: protected_paths.into_iter().collect(),
            staged: BTreeMap::new(),
        }
    }

    #[allow(dead_code)]
    pub fn mode(&self) -> CapabilityMode {
        self.mode
    }

    #[allow(dead_code)]
    pub async fn search(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> KnowledgeResult<Vec<KnowledgeHit>> {
        let limit = limit.unwrap_or(5).clamp(1, 20);
        if query.trim().is_empty() {
            return Err(invalid("query must not be empty"));
        }
        let prefixes = self
            .active_pack_roots()
            .map_err(|error| KnowledgeError::new("internal", error.to_string()))?;
        let citations = crate::retrieval::retrieve(
            &self.state.app_data_dir,
            &self.state,
            query,
            &prefixes,
            limit,
        )
        .await
        .map_err(|error| KnowledgeError::new("internal", format!("retrieval failed: {error}")))?;
        Ok(citations
            .into_iter()
            .map(|citation| KnowledgeHit {
                file_path: citation.file_path,
                title: citation.title,
                snippet: citation.snippet,
                score: citation.score,
            })
            .collect())
    }

    pub fn list(&self, query: Option<&str>) -> KnowledgeResult<KnowledgeListResult> {
        let roots = self
            .active_pack_roots()
            .map_err(|error| KnowledgeError::new("internal", error.to_string()))?;
        let filter = query.unwrap_or("").to_ascii_lowercase();
        let mut files = Vec::new();
        for node in vault::list_tree(&self.state.vault_path())
            .map_err(|error| KnowledgeError::new("internal", error.to_string()))?
        {
            collect_markdown_paths(&node, &roots, &filter, &mut files);
            if files.len() >= MAX_LISTED_FILES {
                break;
            }
        }
        let pending = {
            let conn = self.state.db.lock();
            db::list_pending_chat_file_changes(&conn)
                .map_err(|error| KnowledgeError::new("internal", error.to_string()))?
        };
        for change in pending {
            let path = change.path;
            if change.new_content.is_none() {
                files.retain(|listed| listed != &path);
                continue;
            }
            if roots.iter().any(|root| Path::new(&path).starts_with(root))
                && (filter.is_empty() || path.to_ascii_lowercase().contains(&filter))
                && !files.contains(&path)
            {
                files.push(path);
            }
        }
        files.sort();
        let truncated = files.len() >= MAX_LISTED_FILES;
        files.truncate(MAX_LISTED_FILES);
        Ok(KnowledgeListResult { files, truncated })
    }

    pub fn read(&self, path: &str) -> KnowledgeResult<KnowledgeReadResult> {
        let normalized = normalize_path(path)?;
        let content = self.read_current(&normalized).map_err(|error| {
            if error.to_string().contains("not inside an active pack") {
                KnowledgeError::new(ERR_NOT_FOUND, error.to_string())
            } else {
                error_to_knowledge(error)
            }
        })?;
        let truncated = content.len() > MAX_FILE_BYTES;
        Ok(KnowledgeReadResult {
            path: normalized,
            content: if truncated {
                content.chars().take(MAX_FILE_BYTES).collect::<String>()
            } else {
                content
            },
            truncated,
        })
    }

    pub fn create(&mut self, path: &str, content: &str) -> KnowledgeResult<String> {
        self.stage(path, Some(content.to_string()), StageRequirement::Missing)?;
        Ok(format!("Staged create: {path}"))
    }

    pub fn replace(&mut self, path: &str, content: &str) -> KnowledgeResult<String> {
        self.stage(path, Some(content.to_string()), StageRequirement::Existing)?;
        Ok(format!("Staged replace: {path}"))
    }

    pub fn delete(&mut self, path: &str) -> KnowledgeResult<String> {
        self.stage(path, None, StageRequirement::Existing)?;
        Ok(format!("Staged delete: {path}"))
    }

    #[allow(dead_code)]
    pub fn finish(&self) -> AppResult<Vec<NewChatFileChange>> {
        self.proposals()
    }

    #[allow(dead_code)]
    pub fn abort(&mut self) {
        self.staged.clear();
    }

    fn active_pack_roots(&self) -> AppResult<Vec<String>> {
        let conn = self.state.db.lock();
        Ok(db::list_sync_state(&conn)?
            .into_iter()
            .filter(|pack| pack.active)
            .map(|pack| pack.local_path)
            .collect())
    }

    fn installed_pack_for_path(&self, path: &str) -> AppResult<db::InstalledPack> {
        let candidate = Path::new(path);
        let conn = self.state.db.lock();
        db::list_sync_state(&conn)?
            .into_iter()
            .filter(|pack| pack.active)
            .find(|pack| {
                let root = Path::new(&pack.local_path);
                candidate.starts_with(root) && candidate != root
            })
            .ok_or_else(|| AppError::msg(format!("Path is not inside an active pack: {path}")))
    }

    fn ensure_editable(&self, path: &str) -> AppResult<()> {
        if !vault::is_markdown_path(path) {
            return Err(AppError::msg(
                "Knowledge tools can only edit Markdown (.md) files",
            ));
        }
        if self.protected_paths.contains(path) {
            return Err(AppError::msg(format!(
                "{path} is open in the editor and cannot be changed by the agent"
            )));
        }
        ensure_no_symlink_components(&self.state.vault_path(), path)?;
        let pack = self.installed_pack_for_path(path)?;
        crate::commands::ensure_pack_not_review_locked(&pack)?;
        let user = self
            .state
            .hub_auth
            .lock()
            .as_ref()
            .map(|session| session.user.clone());
        let permitted = match pack.origin.as_str() {
            "local" => true,
            "registry" => user.as_ref().is_some_and(|user| {
                user.role == "admin"
                    || user.role == "superuser"
                    || pack.owner_id.as_deref() == Some(user.id.as_str())
            }),
            _ => false,
        };
        if !permitted {
            return Err(AppError::msg(format!(
                "You do not have edit access to {}",
                pack.name
            )));
        }
        Ok(())
    }

    fn read_current(&self, path: &str) -> AppResult<String> {
        if let Some(staged) = self.staged.get(path) {
            return staged
                .current
                .clone()
                .ok_or_else(|| AppError::msg(format!("{path} is staged for deletion")));
        }
        self.installed_pack_for_path(path)?;
        let pending = {
            let conn = self.state.db.lock();
            db::get_pending_chat_file_change_for_path(&conn, path)?
        };
        if let Some(pending) = pending {
            let disk = vault::read_file(&self.state.vault_path(), path).ok();
            if disk == pending.old_content {
                return pending
                    .new_content
                    .ok_or_else(|| AppError::msg(format!("{path} is pending deletion")));
            }
            if let Ok(crate::knowledge_review::ReconcileOutcome::Rebased) =
                crate::knowledge_review::reconcile_pending_change(&self.state, &pending)
            {
                let conn = self.state.db.lock();
                let rebased = db::get_pending_chat_file_change_for_path(&conn, path)?;
                if let Some(rebased) = rebased {
                    return rebased
                        .new_content
                        .ok_or_else(|| AppError::msg(format!("{path} is pending deletion")));
                }
            }
        }
        vault::read_file(&self.state.vault_path(), path)
    }

    fn stage(
        &mut self,
        path: &str,
        next: Option<String>,
        require: StageRequirement,
    ) -> KnowledgeResult<()> {
        if self.mode != CapabilityMode::Agent {
            return Err(KnowledgeError::new(
                ERR_PERMISSION_DENIED,
                "write capabilities require Agent mode",
            ));
        }
        let path = normalize_path(path)?;
        self.ensure_editable(&path).map_err(error_to_knowledge)?;
        if next
            .as_ref()
            .is_some_and(|content| content.len() > MAX_FILE_BYTES)
        {
            return Err(limit(format!(
                "file size limit exceeded for {path} (256 KiB)"
            )));
        }
        let vault_root = self.state.vault_path();
        let disk_original = vault::read_file(&vault_root, &path).ok();
        let mut pending = {
            let conn = self.state.db.lock();
            db::get_pending_chat_file_change_for_path(&conn, &path)
                .map_err(|error| KnowledgeError::new("internal", error.to_string()))?
        };
        if let Some(existing) = &pending {
            if existing.old_content != disk_original {
                match crate::knowledge_review::reconcile_pending_change(&self.state, existing) {
                    Ok(crate::knowledge_review::ReconcileOutcome::Rebased) => {
                        let conn = self.state.db.lock();
                        pending = db::get_pending_chat_file_change_for_path(&conn, &path)
                            .map_err(|error| KnowledgeError::new("internal", error.to_string()))?;
                    }
                    Ok(_) | Err(_) => {
                        pending = None;
                    }
                }
            }
        }
        let previous = self.staged.get(&path).cloned();
        let effective_exists = previous
            .as_ref()
            .map(|entry| entry.current.is_some())
            .unwrap_or_else(|| {
                pending
                    .as_ref()
                    .map(|change| change.new_content.is_some())
                    .unwrap_or(disk_original.is_some())
            });
        match require {
            StageRequirement::Existing if !effective_exists => {
                return Err(KnowledgeError::new(
                    ERR_NOT_FOUND,
                    format!("{path} does not exist"),
                ));
            }
            StageRequirement::Missing if effective_exists => {
                return Err(KnowledgeError::new(
                    ERR_ALREADY_EXISTS,
                    format!("{path} already exists"),
                ));
            }
            _ => {}
        }
        if !self.staged.contains_key(&path) && self.staged.len() >= MAX_CHANGED_FILES {
            return Err(limit("agent turn may change at most 32 files"));
        }
        let original = self
            .staged
            .get(&path)
            .map(|entry| entry.original.clone())
            .unwrap_or_else(|| {
                pending
                    .as_ref()
                    .map(|change| change.old_content.clone())
                    .unwrap_or(disk_original)
            });
        self.staged.insert(
            path.clone(),
            StagedFile {
                original,
                current: next,
            },
        );
        let total = self
            .staged
            .values()
            .filter_map(|entry| entry.current.as_ref())
            .map(String::len)
            .sum::<usize>();
        if total > MAX_STAGED_BYTES {
            match previous {
                Some(entry) => {
                    self.staged.insert(path, entry);
                }
                None => {
                    self.staged.remove(&path);
                }
            }
            return Err(limit("staged-content limit exceeded (2 MiB)"));
        }
        Ok(())
    }

    fn proposals(&self) -> AppResult<Vec<NewChatFileChange>> {
        let mut changes = self.staged.iter().collect::<Vec<_>>();
        if changes.is_empty() {
            return Ok(Vec::new());
        }
        let root = self.state.vault_path();
        changes.sort_by(|a, b| a.0.cmp(b.0));
        let mut proposals = Vec::with_capacity(changes.len());
        for (path, entry) in changes {
            self.ensure_editable(path)?;
            let disk = vault::read_file(&root, path).ok();
            if let Some(proposal) = finalize_staged_file(path, entry, disk) {
                proposals.push(proposal);
            }
        }
        Ok(proposals)
    }
}

fn operation_for(old: &Option<String>, new: &Option<String>) -> String {
    match (old, new) {
        (None, Some(_)) => "created",
        (Some(_), None) => "deleted",
        _ => "modified",
    }
    .to_string()
}

fn finalize_staged_file(
    path: &str,
    entry: &StagedFile,
    disk: Option<String>,
) -> Option<NewChatFileChange> {
    if disk == entry.current {
        return None;
    }
    if disk == entry.original {
        return Some(NewChatFileChange {
            path: path.to_string(),
            operation: operation_for(&entry.original, &entry.current),
            old_content: entry.original.clone(),
            new_content: entry.current.clone(),
            status: "pending".to_string(),
            rebase_count: 0,
            resolution_reason: None,
        });
    }
    if let (Some(base), Some(proposed), Some(current)) = (&entry.original, &entry.current, &disk) {
        if let crate::knowledge_merge::MergeOutcome::Clean(merged) =
            crate::knowledge_merge::merge_text(base, proposed, current)
        {
            if merged == *current {
                return None;
            }
            return Some(NewChatFileChange {
                path: path.to_string(),
                operation: operation_for(&disk, &Some(merged.clone())),
                old_content: disk,
                new_content: Some(merged),
                status: "pending".to_string(),
                rebase_count: 1,
                resolution_reason: Some(
                    "rebased over a direct workspace change during turn finalization".to_string(),
                ),
            });
        }
    }
    Some(NewChatFileChange {
        path: path.to_string(),
        operation: operation_for(&entry.original, &entry.current),
        old_content: disk,
        new_content: entry.current.clone(),
        status: "conflicted".to_string(),
        rebase_count: 0,
        resolution_reason: Some(
            "staged change overlaps a direct workspace change from the same turn".to_string(),
        ),
    })
}

fn error_to_knowledge(error: AppError) -> KnowledgeError {
    let message = error.to_string();
    let code = if message.contains("not inside an active pack") {
        ERR_NOT_FOUND
    } else if message.contains("only edit Markdown") {
        ERR_INVALID_INPUT
    } else if message.contains("open in the editor") {
        ERR_PROTECTED_PATH
    } else if message.contains("under review") {
        ERR_REVIEW_LOCKED
    } else if message.contains("edit access") {
        ERR_PERMISSION_DENIED
    } else {
        "internal"
    };
    KnowledgeError::new(code, message)
}

pub fn normalize_path(path: &str) -> KnowledgeResult<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(invalid("path must not be empty"));
    }
    if trimmed.contains("..") {
        return Err(invalid("path must not contain parent traversal"));
    }
    Ok(trimmed.replace('\\', "/"))
}

fn ensure_no_symlink_components(root: &Path, rel_path: &str) -> AppResult<()> {
    let mut probe = root.to_path_buf();
    for component in Path::new(rel_path).components() {
        probe.push(component);
        if let Ok(metadata) = std::fs::symlink_metadata(&probe) {
            if metadata.file_type().is_symlink() {
                return Err(AppError::msg(
                    "Knowledge tools cannot edit through symbolic links",
                ));
            }
        }
    }
    Ok(())
}

fn collect_markdown_paths(
    node: &vault::TreeNode,
    roots: &[String],
    query: &str,
    output: &mut Vec<String>,
) {
    if output.len() >= MAX_LISTED_FILES {
        return;
    }
    if matches!(node.kind, vault::TreeNodeKind::File)
        && vault::is_markdown_path(&node.path)
        && roots
            .iter()
            .any(|root| Path::new(&node.path).starts_with(root))
        && (query.is_empty() || node.path.to_ascii_lowercase().contains(query))
    {
        output.push(node.path.clone());
    }
    if let Some(children) = &node.children {
        for child in children {
            collect_markdown_paths(child, roots, query, output);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_rebases_stage_over_non_overlapping_direct_change() {
        let state = test_state_with_pack("merge-pack", "note.md", "one\ntwo\nthree");
        let mut workspace =
            KnowledgeWorkspace::open_turn(state.clone(), CapabilityMode::Agent, Vec::new());
        workspace
            .replace("merge-pack/note.md", "ONE\ntwo\nthree")
            .unwrap();
        std::fs::write(
            state.vault_path().join("merge-pack/note.md"),
            "one\ntwo\nTHREE",
        )
        .unwrap();

        let changes = workspace.finish().unwrap();

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].status, "pending");
        assert_eq!(changes[0].rebase_count, 1);
        assert_eq!(changes[0].old_content.as_deref(), Some("one\ntwo\nTHREE"));
        assert_eq!(changes[0].new_content.as_deref(), Some("ONE\ntwo\nTHREE"));
    }

    #[test]
    fn finish_returns_conflicted_proposal_for_overlapping_direct_change() {
        let state = test_state_with_pack("merge-pack", "note.md", "one\ntwo\nthree");
        let mut workspace =
            KnowledgeWorkspace::open_turn(state.clone(), CapabilityMode::Agent, Vec::new());
        workspace
            .replace("merge-pack/note.md", "one\nPROPOSED\nthree")
            .unwrap();
        std::fs::write(
            state.vault_path().join("merge-pack/note.md"),
            "one\nDIRECT\nthree",
        )
        .unwrap();

        let changes = workspace.finish().unwrap();

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].status, "conflicted");
        assert_eq!(
            changes[0].old_content.as_deref(),
            Some("one\nDIRECT\nthree")
        );
    }

    fn test_state_with_pack(pack: &str, file: &str, content: &str) -> SharedState {
        let state = std::sync::Arc::new(
            crate::state::AppState::new(
                std::env::temp_dir().join(format!("nest-workspace-{}", uuid::Uuid::new_v4())),
            )
            .unwrap(),
        );
        std::fs::create_dir_all(state.vault_path().join(pack)).unwrap();
        std::fs::write(state.vault_path().join(pack).join(file), content).unwrap();
        {
            let conn = state.db.lock();
            db::upsert_sync_state(
                &conn,
                db::SyncStateUpsert {
                    pack_id: pack,
                    name: pack,
                    version: "1.0.0",
                    local_path: pack,
                    origin: "local",
                    owner_id: None,
                    description: "",
                    patch_revision: 0,
                },
            )
            .unwrap();
        }
        state
    }

    #[test]
    fn catalog_matches_mode_capability_matrix() {
        let ask = catalog_for_mode(CapabilityMode::Ask);
        assert_eq!(ask.len(), 3);
        assert!(ask.iter().all(|c| c.mode == CapabilityMode::Ask));

        let agent = catalog_for_mode(CapabilityMode::Agent);
        assert_eq!(agent.len(), 6);
        assert!(capability_allowed(CapabilityMode::Ask, CAPABILITY_SEARCH));
        assert!(capability_allowed(CapabilityMode::Ask, CAPABILITY_READ));
        assert!(!capability_allowed(CapabilityMode::Ask, CAPABILITY_CREATE));
        assert!(!capability_allowed(CapabilityMode::Ask, CAPABILITY_DELETE));
        assert!(capability_allowed(CapabilityMode::Agent, CAPABILITY_CREATE));
        assert!(capability_allowed(CapabilityMode::Agent, CAPABILITY_DELETE));
        assert!(!capability_allowed(CapabilityMode::Ask, "knowledge.rename"));
    }

    #[test]
    fn normalize_path_validates_and_unifies_separators() {
        assert_eq!(normalize_path("a/b.md").unwrap(), "a/b.md");
        assert_eq!(normalize_path(" a\\b.md ").unwrap(), "a/b.md");
        assert!(normalize_path("").is_err());
        assert!(normalize_path("  ").is_err());
        assert!(normalize_path("../x.md").is_err());
        assert!(normalize_path("a/../../x.md").is_err());
    }

    #[test]
    fn knowledge_error_maps_permissions_and_limits() {
        let error = KnowledgeError::new(ERR_PROTECTED_PATH, "locked");
        assert_eq!(error.code, "protected_path");
        assert_eq!(error.to_string(), "protected_path: locked");
    }
}
