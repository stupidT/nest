use crate::claude_cli::{self, ClaudeDetection, ClaudeErrorCode, ProbeOutcome};
use crate::db::{self, ClaudeConnectionReport};
use crate::error::{AppError, AppResult};
use crate::state::SharedState;
use chrono::Utc;
use serde::Deserialize;
use std::time::Duration;
use tauri::State;

const MIN_CONNECTION_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeSettingsRequest {
    pub enabled: bool,
    pub cli_path: String,
    pub custom_models: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeDetectionDto {
    pub configured_path: String,
    pub resolved_path: String,
    pub spawn_strategy: String,
    pub cli_version: Option<String>,
}

#[tauri::command]
pub async fn claude_detect_cli(cli_path: Option<String>) -> AppResult<ClaudeDetectionDto> {
    let configured = cli_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from);
    let detections = claude_cli::detect_cli(configured.as_deref())
        .map_err(|error| AppError::msg(error.to_string()))?;
    for detection in &detections {
        match claude_cli::probe_version(detection, claude_cli::PROBE_VERSION_TIMEOUT).await {
            ProbeOutcome::Version(version) => {
                return Ok(ClaudeDetectionDto {
                    configured_path: detection.configured_path.clone(),
                    resolved_path: detection.resolved_path.clone(),
                    spawn_strategy: spawn_strategy_name(&detection.launch_target),
                    cli_version: Some(version),
                });
            }
            ProbeOutcome::Failed(_) => continue,
        }
    }
    Err(AppError::msg(format!(
        "{}: no candidate produced a version",
        ClaudeErrorCode::InvalidCliPath.as_str()
    )))
}

fn spawn_strategy_name(target: &claude_cli::ClaudeLaunchTarget) -> String {
    match target {
        claude_cli::ClaudeLaunchTarget::Executable { .. } => "direct".to_string(),
        claude_cli::ClaudeLaunchTarget::NodeScript { .. } => "node-script".to_string(),
    }
}

#[tauri::command]
pub async fn claude_test_connection(
    _state: State<'_, SharedState>,
    cli_path: String,
) -> AppResult<ClaudeConnectionReport> {
    let report = test_connection(&cli_path).await;
    Ok(report)
}

#[tauri::command]
pub async fn claude_save_settings(
    state: State<'_, SharedState>,
    request: ClaudeSettingsRequest,
) -> AppResult<ClaudeConnectionReport> {
    {
        let conn = state.db.lock();
        db::save_claude_settings(
            &conn,
            request.enabled,
            &request.cli_path,
            &request.custom_models,
        )?;
    }
    if !request.enabled {
        state.claude_connection.lock().take();
        return Ok(ClaudeConnectionReport {
            connected: false,
            configured_cli_path: request.cli_path,
            ..Default::default()
        });
    }
    let report = test_connection(&request.cli_path).await;
    *state.claude_connection.lock() = Some(report.clone());
    Ok(report)
}

#[tauri::command]
pub fn claude_connection_status(
    state: State<'_, SharedState>,
) -> AppResult<ClaudeConnectionReport> {
    let settings = {
        let conn = state.db.lock();
        db::get_settings(&conn)?
    };
    let stored = state.claude_connection.lock().clone();
    Ok(match stored {
        Some(report) if report.is_connected(&settings.claude_cli_path) => report,
        _ => ClaudeConnectionReport {
            connected: false,
            configured_cli_path: settings.claude_cli_path,
            ..Default::default()
        },
    })
}

async fn test_connection(cli_path: &str) -> ClaudeConnectionReport {
    let trimmed = cli_path.trim();
    let configured = if trimmed.is_empty() {
        None
    } else {
        Some(std::path::PathBuf::from(trimmed))
    };
    let detections = match claude_cli::detect_cli(configured.as_deref()) {
        Ok(detections) => detections,
        Err(error) => {
            return failure_report(trimmed, &error.to_string());
        }
    };

    for detection in &detections {
        match claude_cli::probe_version(detection, claude_cli::PROBE_VERSION_TIMEOUT).await {
            ProbeOutcome::Version(_) => {
                if let Some(report) = minimal_round_trip(detection, trimmed).await {
                    return report;
                }
            }
            ProbeOutcome::Failed(_) => continue,
        }
    }
    failure_report(trimmed, "no CLI candidate completed the connection test")
}

async fn minimal_round_trip(
    detection: &ClaudeDetection,
    configured_path: &str,
) -> Option<ClaudeConnectionReport> {
    let temp_dir = std::env::temp_dir();
    let probe_session = uuid::Uuid::new_v4().to_string();
    let outcome =
        claude_cli::probe_connection(detection, &probe_session, &temp_dir, MIN_CONNECTION_TIMEOUT)
            .await;
    match outcome {
        Ok(result) => Some(ClaudeConnectionReport {
            connected: true,
            configured_cli_path: configured_path.to_string(),
            resolved_cli_path: result.resolved_path,
            cli_version: result.cli_version,
            effective_model: result.effective_model,
            tested_at: Utc::now().to_rfc3339(),
            message: None,
        }),
        Err(message) => Some(failure_report(configured_path, &message)),
    }
}

fn failure_report(cli_path: &str, message: &str) -> ClaudeConnectionReport {
    ClaudeConnectionReport {
        connected: false,
        configured_cli_path: cli_path.to_string(),
        message: Some(message.to_string()),
        ..Default::default()
    }
}
