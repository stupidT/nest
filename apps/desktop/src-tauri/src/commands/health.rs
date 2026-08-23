use crate::error::AppResult;
use crate::state::SharedState;
use tauri::State;

#[tauri::command]
pub fn workspace_health(
    state: State<'_, SharedState>,
) -> AppResult<crate::vault_reconciliation::WorkspaceHealth> {
    Ok(crate::vault_reconciliation::load_health(&state))
}

#[tauri::command]
pub fn workspace_reindex(
    state: State<'_, SharedState>,
) -> AppResult<crate::vault_reconciliation::WorkspaceHealth> {
    crate::indexing::schedule(&state)?;
    crate::vault_reconciliation::clear_reindex_required(&state)?;
    Ok(crate::vault_reconciliation::load_health(&state))
}
