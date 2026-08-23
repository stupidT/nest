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
pub async fn workspace_reindex(
    state: State<'_, SharedState>,
) -> AppResult<crate::vault_reconciliation::WorkspaceHealth> {
    crate::vault_reconciliation::restore_workspace(&state, std::time::Duration::from_secs(300))
        .await
}
