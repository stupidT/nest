use crate::db::{self, NewChatFileChange};
use crate::error::{AppError, AppResult};
use crate::state::SharedState;
use crate::vault;
use std::path::Path;

#[allow(dead_code)]
pub const REVIEW_STATUS_PENDING: &str = "pending";
#[allow(dead_code)]
pub const REVIEW_STATUS_APPLYING: &str = "applying";
pub const REVIEW_STATUS_APPROVED: &str = "approved";
pub const REVIEW_STATUS_REJECTED: &str = "rejected";
#[allow(dead_code)]
pub const REVIEW_STATUS_FAILED: &str = "failed";

pub enum ReviewOutcome {
    Approved,
    Rejected,
    Failed { code: String, message: String },
}

pub struct KnowledgeReview;

impl KnowledgeReview {
    pub fn review(state: &SharedState, change_id: &str, approve: bool) -> AppResult<ReviewOutcome> {
        let change = {
            let conn = state.db.lock();
            let change = db::get_chat_file_change(&conn, change_id)?;
            if change.status != REVIEW_STATUS_PENDING {
                return Err(AppError::msg("File change is no longer pending"));
            }
            if !approve {
                db::set_chat_file_change_status(&conn, change_id, REVIEW_STATUS_REJECTED)?;
                return Ok(ReviewOutcome::Rejected);
            }
            let claim_id = uuid::Uuid::new_v4().to_string();
            db::claim_chat_file_change(&conn, change_id, &claim_id)?;
            change
        };

        let applied = Self::apply_change(state, &change);
        let result = {
            let conn = state.db.lock();
            match &applied {
                Ok(()) => db::set_chat_file_change_status(&conn, change_id, REVIEW_STATUS_APPROVED),
                Err(error) => {
                    db::fail_chat_file_change(&conn, change_id, "apply_failed", &error.to_string())
                }
            }
        };
        if let Err(status_error) = result {
            if applied.is_ok() {
                rollback_files(
                    &state.vault_path(),
                    &[(change.path.clone(), change.old_content.clone())],
                );
            }
            let conn = state.db.lock();
            let _ = db::clear_chat_file_change_claim(&conn, change_id);
            return Ok(ReviewOutcome::Failed {
                code: "apply_status_failed".to_string(),
                message: status_error.to_string(),
            });
        }
        match applied {
            Ok(()) => {
                {
                    let conn = state.db.lock();
                    db::clear_chat_file_change_claim(&conn, change_id)?;
                }
                let _ = crate::indexing::schedule(state);
                Ok(ReviewOutcome::Approved)
            }
            Err(error) => Ok(ReviewOutcome::Failed {
                code: "apply_failed".to_string(),
                message: error.to_string(),
            }),
        }
    }

    fn apply_change(state: &SharedState, change: &db::ChatFileChangeDetail) -> AppResult<()> {
        let root = state.vault_path();
        ensure_editable(state, &change.path)?;
        let current = vault::read_file(&root, &change.path).ok();
        if current != change.old_content {
            return Err(AppError::msg(format!(
                "{} changed after the proposal was created; review it again before applying",
                change.path
            )));
        }
        match &change.new_content {
            Some(content) => vault::write_file(&root, &change.path, content),
            None => vault::delete_file(&root, &change.path),
        }
    }
}

#[allow(dead_code)]
pub fn rollback_changes(state: &SharedState, changes: &[NewChatFileChange]) {
    let originals = changes
        .iter()
        .map(|change| (change.path.clone(), change.old_content.clone()))
        .collect::<Vec<_>>();
    rollback_files(&state.vault_path(), &originals);
}

fn rollback_files(root: &Path, files: &[(String, Option<String>)]) {
    for (path, original) in files.iter().rev() {
        match original {
            Some(content) => {
                let _ = vault::write_file(root, path, content);
            }
            None => {
                let _ = vault::delete_file(root, path);
            }
        }
    }
}

fn ensure_editable(state: &SharedState, path: &str) -> AppResult<()> {
    if !vault::is_markdown_path(path) {
        return Err(AppError::msg(
            "Knowledge tools can only edit Markdown (.md) files",
        ));
    }
    ensure_no_symlink_components(&state.vault_path(), path)?;
    let candidate = Path::new(path);
    let pack = {
        let conn = state.db.lock();
        db::list_sync_state(&conn)?
            .into_iter()
            .filter(|pack| pack.active)
            .find(|pack| {
                let root = Path::new(&pack.local_path);
                candidate.starts_with(root) && candidate != root
            })
            .ok_or_else(|| AppError::msg(format!("Path is not inside an active pack: {path}")))?
    };
    crate::commands::ensure_pack_not_review_locked(&pack)?;
    let user = state
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
