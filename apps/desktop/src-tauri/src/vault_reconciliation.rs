use crate::db;
use crate::error::AppResult;
use crate::state::SharedState;
use std::collections::HashMap;
use std::path::Path;

pub const WORKSPACE_HEALTH_KEY: &str = "workspace_health_v1";

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceHealth {
    pub reindex_required: bool,
    pub reason: Option<String>,
    pub updated_at: Option<String>,
}

pub fn load_health(state: &SharedState) -> WorkspaceHealth {
    let conn = state.db.lock();
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        rusqlite::params![WORKSPACE_HEALTH_KEY],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|value: String| serde_json::from_str(&value).ok())
    .unwrap_or_default()
}

fn save_health(state: &SharedState, health: &WorkspaceHealth) -> AppResult<()> {
    let conn = state.db.lock();
    conn.execute(
        "INSERT INTO settings(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![WORKSPACE_HEALTH_KEY, serde_json::to_string(health)?],
    )?;
    Ok(())
}

pub fn set_reindex_required(state: &SharedState, reason: &str) -> AppResult<()> {
    let mut health = load_health(state);
    health.reindex_required = true;
    health.reason = Some(reason.to_string());
    health.updated_at = Some(chrono::Utc::now().to_rfc3339());
    save_health(state, &health)
}

pub fn clear_reindex_required(state: &SharedState) -> AppResult<()> {
    let mut health = load_health(state);
    health.reindex_required = false;
    health.reason = None;
    health.updated_at = Some(chrono::Utc::now().to_rfc3339());
    save_health(state, &health)
}

pub struct ReconcileReport {
    pub rebased: usize,
    pub conflicted: usize,
    pub resolved_external: usize,
    pub reindex_required: bool,
}

fn snapshot_manifest(vault_root: &Path, prefixes: &[String]) -> AppResult<HashMap<String, u64>> {
    let mut manifest = HashMap::new();
    for prefix in prefixes {
        let root = vault_root.join(prefix);
        if !root.is_dir() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&root) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let rel = match path.strip_prefix(vault_root) {
                Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            if !crate::vault::is_markdown_path(&rel) {
                continue;
            }
            let hash = match std::fs::read(path) {
                Ok(bytes) => {
                    use std::hash::{Hash, Hasher};
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    bytes.hash(&mut hasher);
                    hasher.finish()
                }
                Err(_) => continue,
            };
            manifest.insert(rel, hash);
        }
    }
    Ok(manifest)
}

pub fn reconcile_vault(state: &SharedState) -> AppResult<ReconcileReport> {
    let vault_root = state.vault_path();
    let pending_changes = {
        let conn = state.db.lock();
        db::list_pending_chat_file_changes(&conn)?
    };
    let mut report = ReconcileReport {
        rebased: 0,
        conflicted: 0,
        resolved_external: 0,
        reindex_required: false,
    };
    for change in &pending_changes {
        match crate::knowledge_review::reconcile_pending_change(state, change) {
            Ok(crate::knowledge_review::ReconcileOutcome::Rebased) => report.rebased += 1,
            Ok(crate::knowledge_review::ReconcileOutcome::Conflicted) => report.conflicted += 1,
            Ok(crate::knowledge_review::ReconcileOutcome::ResolvedExternal) => {
                report.resolved_external += 1
            }
            Ok(crate::knowledge_review::ReconcileOutcome::Unchanged) => {}
            Err(_) => {
                report.conflicted += 1;
            }
        }
    }

    let prefixes: Vec<String> = {
        let conn = state.db.lock();
        db::list_sync_state(&conn)?
            .into_iter()
            .filter(|pack| pack.active)
            .map(|pack| pack.local_path)
            .collect()
    };
    let current = snapshot_manifest(&vault_root, &prefixes)?;
    let indexed_files: HashMap<String, u64> = {
        let conn = state.db.lock();
        let mut stmt = conn.prepare("SELECT file_path FROM chunks")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut map = HashMap::new();
        for path in rows.flatten() {
            map.insert(path, 0);
        }
        map
    };
    let mut missing_from_index = 0usize;
    for path in current.keys() {
        if !indexed_files.contains_key(path) {
            missing_from_index += 1;
        }
    }
    let index_meta: (u32, u32) = {
        let conn = state.db.lock();
        conn.query_row(
            "SELECT indexed_files, indexed_chunks FROM index_meta WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap_or((0, 0))
    };
    let _ = index_meta;
    if missing_from_index > 0 {
        crate::indexing::schedule(state)?;
    }

    Ok(report)
}

pub fn ensure_workspace_healthy(state: &SharedState) -> AppResult<()> {
    let health = load_health(state);
    if health.reindex_required {
        return Err(crate::error::AppError::msg(format!(
            "nest_knowledge_reindex_required: {}",
            health
                .reason
                .unwrap_or_else(|| "index rebuild required".into())
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn reindex_flag_round_trips_and_clears() {
        let state = Arc::new(
            crate::state::AppState::new(
                std::env::temp_dir().join(format!("nest-health-{}", uuid::Uuid::new_v4())),
            )
            .unwrap(),
        );
        assert!(!load_health(&state).reindex_required);
        set_reindex_required(&state, "scan failed").unwrap();
        let health = load_health(&state);
        assert!(health.reindex_required);
        assert_eq!(health.reason.as_deref(), Some("scan failed"));
        ensure_workspace_healthy(&state).unwrap_err();
        clear_reindex_required(&state).unwrap();
        assert!(!load_health(&state).reindex_required);
        ensure_workspace_healthy(&state).unwrap();
    }
}
