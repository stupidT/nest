use crate::claude_mcp::{start_server, McpServerState, ToolEventSink};
use crate::knowledge_workspace::CapabilityMode;
use crate::state::SharedState;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub struct ProbeOutcome {
    #[allow(dead_code)]
    pub tools_exercised: Vec<String>,
    pub failures: Vec<String>,
    pub cleanup_warnings: Vec<String>,
}

pub type SinkBuilder = Box<dyn FnOnce() -> ToolEventSink + Send>;

#[allow(dead_code)]
const PROBE_TIMEOUT: Duration = Duration::from_secs(300);

pub async fn run_six_tool_probe(
    state: SharedState,
    sink_builder: Option<SinkBuilder>,
    cli_path: Option<String>,
) -> ProbeOutcome {
    match cli_path.as_deref() {
        Some(path) => run_claude_driven_probe(state, sink_builder, path).await,
        None => crate::connection_probe_direct::run_direct_probe(state, sink_builder).await,
    }
}

struct ProbeEnv {
    probe_session: String,
    probe_turn_id: String,
    pack_dir: String,
    probe_path: String,
    challenge: String,
    config_path: PathBuf,
}

async fn run_claude_driven_probe(
    state: SharedState,
    sink_builder: Option<SinkBuilder>,
    cli_path: &str,
) -> ProbeOutcome {
    let detections =
        match crate::claude_cli::detect_cli(Some(std::path::Path::new(cli_path.trim()))) {
            Ok(detections) if !detections.is_empty() => detections,
            _ => {
                return ProbeOutcome {
                    tools_exercised: Vec::new(),
                    failures: vec!["invalid_cli_path: CLI not found for probe".to_string()],
                    cleanup_warnings: Vec::new(),
                };
            }
        };
    let detection = detections[0].clone();

    let server = McpServerState::new(state.clone());
    if let Some(builder) = sink_builder {
        server.set_event_sink(builder());
    }
    let handle = match start_server(server.clone()).await {
        Ok(handle) => handle,
        Err(error) => {
            return ProbeOutcome {
                tools_exercised: Vec::new(),
                failures: vec![format!("mcp server start failed: {error}")],
                cleanup_warnings: Vec::new(),
            };
        }
    };

    let probe_session = uuid::Uuid::new_v4().to_string();
    let pack_dir = format!("__probe_{probe_session}");
    let pack_root = state.vault_path().join(&pack_dir);
    if let Err(error) = std::fs::create_dir_all(&pack_root) {
        let _ = handle.stop().await;
        return ProbeOutcome {
            tools_exercised: Vec::new(),
            failures: vec![format!("probe pack creation failed: {error}")],
            cleanup_warnings: Vec::new(),
        };
    }
    let registration = {
        let conn = state.db.lock();
        crate::db::upsert_sync_state(
            &conn,
            crate::db::SyncStateUpsert {
                pack_id: &pack_dir,
                name: "Nest Connection Probe",
                version: "1.0.0",
                local_path: &pack_dir,
                origin: "local",
                owner_id: None,
                description: "",
                patch_revision: 0,
            },
        )
    };
    if let Err(error) = registration {
        let _ = handle.stop().await;
        return ProbeOutcome {
            tools_exercised: Vec::new(),
            failures: vec![format!("probe pack registration failed: {error}")],
            cleanup_warnings: Vec::new(),
        };
    }

    let probe_path = format!("{pack_dir}/probe.md");
    let challenge = format!("nest-probe-{}", uuid::Uuid::new_v4().simple());
    let probe_turn_id = uuid::Uuid::new_v4().to_string();
    {
        let conn = state.db.lock();
        let _ = conn.execute(
            "INSERT INTO chat_sessions (id, title, title_source, mode, created_at, updated_at)
             VALUES (?1, 'Nest Connection Probe', 'placeholder', 'agent', ?2, ?2)",
            rusqlite::params![probe_session, chrono::Utc::now().to_rfc3339()],
        );
        let _ = conn.execute(
            "INSERT INTO chat_messages (id, session_id, role, content, citations_json, created_at)
             VALUES (?1, ?2, 'user', 'probe', '', ?3)",
            rusqlite::params![
                uuid::Uuid::new_v4().to_string(),
                probe_session,
                chrono::Utc::now().to_rfc3339()
            ],
        );
        let _ = conn.execute(
            "INSERT INTO chat_turns (id, session_id, user_message_id, backend_id,
                requested_model_kind, mode, selection_revision, status, started_at)
             SELECT ?1, ?2, id, 'claude', 'default', 'agent', 0, 'running', ?3
             FROM chat_messages WHERE session_id = ?2 ORDER BY created_at DESC LIMIT 1",
            rusqlite::params![
                probe_turn_id,
                probe_session,
                chrono::Utc::now().to_rfc3339()
            ],
        );
    }
    let credential = match server.begin_turn(
        &probe_session,
        &probe_turn_id,
        CapabilityMode::Agent,
        Vec::new(),
    ) {
        Ok(credential) => credential,
        Err(error) => {
            cleanup(&state, &pack_dir, &probe_session, "", handle.clone()).await;
            return ProbeOutcome {
                tools_exercised: Vec::new(),
                failures: vec![error],
                cleanup_warnings: Vec::new(),
            };
        }
    };
    let config_path = std::env::temp_dir().join(format!(
        "nest-probe-mcp-{}.json",
        uuid::Uuid::new_v4().simple()
    ));
    if let Err(error) = std::fs::write(&config_path, handle.config_json(&credential)) {
        cleanup(&state, &pack_dir, &probe_session, "", handle.clone()).await;
        return ProbeOutcome {
            tools_exercised: Vec::new(),
            failures: vec![format!("probe mcp config write failed: {error}")],
            cleanup_warnings: Vec::new(),
        };
    }

    let env = ProbeEnv {
        probe_session,
        probe_turn_id,
        pack_dir,
        probe_path,
        challenge,
        config_path,
    };

    let turn1_prompt = format!(
        "Use the Nest knowledge tools to do exactly these steps, in order:\n\
         1. knowledge_create at path {path} with this exact marker on its own line: {marker}\n\
         2. knowledge_list filtered to {pack}\n\
         3. knowledge_read {path}\n\
         4. knowledge_replace {path} changing the marker line to {marker2}\n\
         Use only the Nest MCP tools (mcp__nest__*), not your native file tools. Reply with a one-line confirmation of each step.",
        path = env.probe_path,
        marker = env.challenge,
        marker2 = format_args!("{}-v2", env.challenge),
        pack = env.pack_dir,
    );

    let turn1 = crate::claude_cli::run_turn(
        &detection,
        crate::claude_cli::ClaudeTurnRequest {
            vault_root: &state.vault_path(),
            session_id: &env.probe_session,
            mode: crate::claude_cli::TurnMode::NewSession,
            prompt: &turn1_prompt,
            model: None,
            chat_mode: CapabilityMode::Agent,
            mcp_config_path: Some(env.config_path.as_path()),
            system_instructions: None,
        },
        &crate::claude_cli::TurnEvents::default(),
        &never_cancel(),
    )
    .await;
    if let Err(error) = turn1 {
        server.end_turn();
        server.clear_event_sink();
        let outcome = cleanup(
            &state,
            &env.pack_dir,
            &env.probe_session,
            &env.probe_turn_id,
            handle.clone(),
        )
        .await;
        return ProbeOutcome {
            tools_exercised: Vec::new(),
            failures: vec![format!("probe turn 1 failed: {error}")],
            cleanup_warnings: outcome,
        };
    }

    let turn2_prompt = format!(
        "Using the Nest knowledge tools only:\n\
         1. knowledge_search for the marker {marker}\n\
         2. knowledge_read {path} to confirm the current marker\n\
         3. knowledge_delete {path}\n\
         Reply with a one-line confirmation of each step.",
        marker = format_args!("{}-v2", env.challenge),
        path = env.probe_path,
    );

    let turn2 = crate::claude_cli::run_turn(
        &detection,
        crate::claude_cli::ClaudeTurnRequest {
            vault_root: &state.vault_path(),
            session_id: &env.probe_session,
            mode: crate::claude_cli::TurnMode::Resume,
            prompt: &turn2_prompt,
            model: None,
            chat_mode: CapabilityMode::Agent,
            mcp_config_path: Some(env.config_path.as_path()),
            system_instructions: None,
        },
        &crate::claude_cli::TurnEvents::default(),
        &never_cancel(),
    )
    .await;

    let staged = server.finish_staged().unwrap_or_default();
    server.end_turn();
    server.clear_event_sink();
    let _ = std::fs::remove_file(&env.config_path);

    let mut failures = Vec::new();
    if let Err(error) = turn2 {
        failures.push(format!("probe turn 2 failed: {error}"));
    }

    let called = {
        let conn = state.db.lock();
        crate::db::list_tool_activities(&conn, &env.probe_turn_id)
            .map(|rows| {
                rows.into_iter()
                    .map(|row| (row.label, row.status))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let required = [
        "knowledge_create",
        "knowledge_list",
        "knowledge_read",
        "knowledge_replace",
        "knowledge_search",
        "knowledge_delete",
    ];
    let mut tools_exercised = Vec::new();
    for tool in required {
        let prefixed = format!("mcp__nest__{tool}");
        let seen = called
            .iter()
            .any(|(label, status)| (label == tool || label == &prefixed) && status == "succeeded");
        if seen {
            tools_exercised.push(prefixed);
        } else {
            failures.push(format!("nest tool not exercised by Claude: {tool}"));
        }
    }

    let probe_file = state.vault_path().join(&env.probe_path);
    if probe_file.exists() && !staged.is_empty() {
        failures.push("probe file still present after staged delete".to_string());
    }

    let disk_after = std::fs::read_to_string(&probe_file).ok();
    if tools_exercised.len() == 6 {
        let marker_seen = staged.iter().any(|change| {
            change
                .new_content
                .as_deref()
                .map(|c| c.contains(&env.challenge))
                .unwrap_or(false)
        }) || disk_after
            .as_deref()
            .map(|content| content.contains(&env.challenge))
            .unwrap_or(false);
        if !marker_seen {
            failures.push("probe marker not found in staged or disk content".to_string());
        }
    }

    let native_reads = called
        .iter()
        .filter(|(label, _)| {
            !label.starts_with("mcp__nest__")
                && !label.starts_with("knowledge_")
                && matches!(
                    label.to_ascii_lowercase().as_str(),
                    "read" | "edit" | "write" | "bash"
                )
        })
        .count();
    if native_reads > 0 {
        failures.push(format!(
            "nest_tool_route_bypassed: Claude used native file tools {native_reads} times during the probe"
        ));
    }

    let cleanup_warnings = cleanup(
        &state,
        &env.pack_dir,
        &env.probe_session,
        &env.probe_turn_id,
        handle.clone(),
    )
    .await;

    ProbeOutcome {
        tools_exercised,
        failures,
        cleanup_warnings,
    }
}

fn never_cancel() -> crate::claude_cli::CancelToken {
    Arc::new(std::sync::atomic::AtomicBool::new(false))
}

async fn cleanup(
    state: &SharedState,
    pack_dir: &str,
    probe_session: &str,
    probe_turn_id: &str,
    handle: crate::claude_mcp::McpServerHandle,
) -> Vec<String> {
    let mut warnings = Vec::new();
    let root = state.vault_path().join(pack_dir);
    if root.exists() {
        if let Err(error) = std::fs::remove_dir_all(&root) {
            warnings.push(format!("probe cleanup failed: {error}"));
        }
    }
    {
        let conn = state.db.lock();
        if let Err(error) = crate::db::purge_path_data(&conn, pack_dir) {
            warnings.push(format!("probe pack deregistration failed: {error}"));
        }
        let _ = crate::db::finalize_running_tool_activities(&conn, probe_turn_id, "succeeded");
        let _ = conn.execute(
            "DELETE FROM chat_sessions WHERE id = ?1",
            rusqlite::params![probe_session],
        );
    }
    handle.stop().await;
    warnings
}
