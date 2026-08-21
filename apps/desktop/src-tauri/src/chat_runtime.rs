use crate::agent::{AgentChatRequest, AgentChatResult};
use crate::chat_events::ChatStreamEvent;
use crate::claude_cli::{self, ClaudeTurnRequest, ClaudeTurnResult, TurnEvents, TurnMode};
use crate::db::{self, ChatBackend, ChatBackendStatus, ChatSession};
use crate::state::SharedState;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

pub struct ChatRunRequest {
    pub app: AppHandle,
    pub state: SharedState,
    pub app_data_dir: PathBuf,
    pub settings: db::AppSettings,
    pub session: ChatSession,
    pub query: String,
    pub focus_paths: Vec<String>,
    pub prior_history: Vec<rig::completion::Message>,
    pub mode: String,
    pub protected_paths: Vec<String>,
    pub stream_event: String,
}

pub struct ChatRunResult {
    pub answer: String,
    pub citations: Vec<db::Citation>,
    pub thinking: Option<String>,
    pub thinking_seconds: Option<f64>,
    pub file_changes: Vec<db::NewChatFileChange>,
    pub backend: ChatBackend,
}

pub fn claude_mode_for(session: &ChatSession) -> Option<TurnMode> {
    match (session.backend, session.backend_status) {
        (Some(ChatBackend::Claude), ChatBackendStatus::Uninitialized) => Some(TurnMode::NewSession),
        (Some(ChatBackend::Claude), ChatBackendStatus::Ready) => Some(TurnMode::Resume),
        _ => None,
    }
}

pub fn claude_available(settings: &db::AppSettings) -> bool {
    settings.claude_agent_enabled
}

pub async fn run_chat(request: ChatRunRequest) -> Result<ChatRunResult, crate::error::AppError> {
    match request.session.backend {
        Some(ChatBackend::Nest) => run_nest(request).await,
        Some(ChatBackend::Claude) => run_claude(request).await,
        None => Err(crate::error::AppError::msg(
            "chat_runtime: session backend is not bound",
        )),
    }
}

async fn run_nest(request: ChatRunRequest) -> Result<ChatRunResult, crate::error::AppError> {
    let ChatRunRequest {
        app,
        state,
        app_data_dir,
        settings,
        session,
        query,
        focus_paths,
        prior_history,
        mode,
        protected_paths,
        stream_event,
    } = request;
    let result: AgentChatResult = crate::agent::run_agent_chat(AgentChatRequest {
        app,
        state,
        app_data_dir,
        settings,
        session_id: session.id,
        query,
        focus_paths,
        stream_event,
        prior_history,
        mode,
        protected_paths,
    })
    .await?;
    Ok(ChatRunResult {
        answer: result.answer,
        citations: result.citations,
        thinking: result.thinking,
        thinking_seconds: result.thinking_seconds,
        file_changes: result.file_changes,
        backend: ChatBackend::Nest,
    })
}

async fn run_claude(request: ChatRunRequest) -> Result<ChatRunResult, crate::error::AppError> {
    let ChatRunRequest {
        app,
        state,
        settings,
        session,
        query,
        stream_event,
        ..
    } = request;

    let Some(turn_mode) = claude_mode_for(&session) else {
        return Err(crate::error::AppError::msg(
            "Claude session is unresumable; start a new chat",
        ));
    };
    let configured = if settings.claude_cli_path.trim().is_empty() {
        None
    } else {
        Some(PathBuf::from(settings.claude_cli_path.trim()))
    };
    let detections = claude_cli::detect_cli(configured.as_deref())
        .map_err(|error| crate::error::AppError::msg(error.to_string()))?;
    let detection = detections
        .first()
        .ok_or_else(|| crate::error::AppError::msg("claude_cli: no CLI candidate resolved"))?;

    let vault_root = state.vault_path();
    let session_id = session.id.clone();
    let state_for_init = state.clone();
    let app_token = app.clone();
    let stream_token = stream_event.clone();
    let app_thinking = app.clone();
    let stream_thinking = stream_event.clone();
    let events = TurnEvents {
        token: Box::new(move |text| {
            let _ = app_token.emit(
                &stream_token,
                ChatStreamEvent::Token {
                    content: text.to_string(),
                },
            );
        }),
        thinking: Box::new(move |text| {
            let _ = app_thinking.emit(
                &stream_thinking,
                ChatStreamEvent::Thinking {
                    content: text.to_string(),
                },
            );
        }),
        initialized: Box::new(move |session_id, _model, _version| {
            let conn = state_for_init.db.lock();
            db::set_session_backend_status(&conn, session_id, ChatBackendStatus::Ready)
                .map_err(|error| error.to_string())
                .map(|_| ())
        }),
    };

    let turn_request = ClaudeTurnRequest {
        vault_root: &vault_root,
        session_id: &session_id,
        mode: turn_mode,
        prompt: &query,
    };
    let cancel = state.begin_chat_cancel_arc();
    let result: ClaudeTurnResult = claude_cli::run_turn(detection, turn_request, &events, &cancel)
        .await
        .map_err(map_turn_error)?;

    Ok(ChatRunResult {
        answer: result.answer,
        citations: Vec::new(),
        thinking: (!result.thinking.trim().is_empty()).then_some(result.thinking),
        thinking_seconds: None,
        file_changes: Vec::new(),
        backend: ChatBackend::Claude,
    })
}

fn map_turn_error(error: claude_cli::ClaudeTurnError) -> crate::error::AppError {
    match error {
        claude_cli::ClaudeTurnError::Cancelled => crate::error::AppError::msg("cancelled"),
        claude_cli::ClaudeTurnError::SpawnFailed { message } => {
            crate::error::AppError::msg(format!("claude_process_failed: {message}"))
        }
        claude_cli::ClaudeTurnError::Io { message } => {
            crate::error::AppError::msg(format!("claude_process_failed: {message}"))
        }
        claude_cli::ClaudeTurnError::InitPersist { message } => {
            crate::error::AppError::msg(format!("claude_process_failed: {message}"))
        }
        claude_cli::ClaudeTurnError::Protocol { message } => {
            crate::error::AppError::msg(format!("claude_protocol_error: {message}"))
        }
        claude_cli::ClaudeTurnError::SessionMismatch { message } => {
            crate::error::AppError::msg(format!("claude_session_mismatch: {message}"))
        }
        claude_cli::ClaudeTurnError::Process { stderr_tail, .. } => {
            let tail = if stderr_tail.is_empty() {
                String::new()
            } else {
                format!(
                    " | stderr: {}",
                    stderr_tail.chars().take(200).collect::<String>()
                )
            };
            crate::error::AppError::msg(format!("claude_process_failed{tail}"))
        }
        claude_cli::ClaudeTurnError::CliError {
            code,
            subtype,
            sanitized_result,
            ..
        } => {
            let detail = match (subtype.as_str(), sanitized_result.as_deref()) {
                ("", None) => String::new(),
                ("", Some(text)) => format!(": {text}"),
                (subtype, None) => format!(": {subtype}"),
                (subtype, Some(text)) => format!(": {subtype}: {text}"),
            };
            crate::error::AppError::msg(format!("{code}{detail}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_mode_follows_backend_status() {
        let uninitialized = db::ChatSession {
            backend: Some(ChatBackend::Claude),
            backend_status: ChatBackendStatus::Uninitialized,
            ..test_session()
        };
        assert_eq!(claude_mode_for(&uninitialized), Some(TurnMode::NewSession));
        let ready = db::ChatSession {
            backend: Some(ChatBackend::Claude),
            backend_status: ChatBackendStatus::Ready,
            ..test_session()
        };
        assert_eq!(claude_mode_for(&ready), Some(TurnMode::Resume));
        let unresumable = db::ChatSession {
            backend: Some(ChatBackend::Claude),
            backend_status: ChatBackendStatus::Unresumable,
            ..test_session()
        };
        assert_eq!(claude_mode_for(&unresumable), None);
        let nest = db::ChatSession {
            backend: Some(ChatBackend::Nest),
            backend_status: ChatBackendStatus::Ready,
            ..test_session()
        };
        assert_eq!(claude_mode_for(&nest), None);
        let unbound = db::ChatSession {
            backend: None,
            backend_status: ChatBackendStatus::Uninitialized,
            ..test_session()
        };
        assert_eq!(claude_mode_for(&unbound), None);
    }

    #[test]
    fn claude_availability_follows_the_toggle() {
        let mut settings = db::AppSettings::default();
        assert!(!claude_available(&settings));
        settings.claude_agent_enabled = true;
        assert!(claude_available(&settings));
    }

    #[test]
    fn turn_errors_map_to_stable_error_codes() {
        let error = map_turn_error(claude_cli::ClaudeTurnError::Cancelled);
        assert_eq!(error.to_string(), "cancelled");
        let error = map_turn_error(claude_cli::ClaudeTurnError::Protocol {
            message: "bad stream".to_string(),
        });
        assert_eq!(error.to_string(), "claude_protocol_error: bad stream");
        let error = map_turn_error(claude_cli::ClaudeTurnError::Process {
            stderr_tail: "x".repeat(600),
            id_in_use: false,
            no_conversation: false,
            saw_init: false,
        });
        let text = error.to_string();
        assert!(text.starts_with("claude_process_failed | stderr: "));
        assert!(text.chars().count() < 260);
        let error = map_turn_error(claude_cli::ClaudeTurnError::CliError {
            code: claude_cli::ClaudeErrorCode::Protocol,
            subtype: "error_during_execution".to_string(),
            sanitized_result: Some("boom".to_string()),
            exit_ok: true,
        });
        assert_eq!(
            error.to_string(),
            "claude_protocol_error: error_during_execution: boom"
        );
    }

    fn test_session() -> db::ChatSession {
        db::ChatSession {
            id: "11111111-2222-4333-8444-555555555555".to_string(),
            title: "Test".to_string(),
            pinned: false,
            archived: false,
            title_source: "placeholder".to_string(),
            mode: "ask".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            backend: None,
            backend_status: ChatBackendStatus::Uninitialized,
        }
    }
}
