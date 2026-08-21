use crate::error::AppError;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeErrorCode {
    InvalidCliPath,
    NodeNotFound,
    Protocol,
    SessionMismatch,
    ProcessFailed,
}

impl ClaudeErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ClaudeErrorCode::InvalidCliPath => "invalid_cli_path",
            ClaudeErrorCode::NodeNotFound => "node_not_found",
            ClaudeErrorCode::Protocol => "claude_protocol_error",
            ClaudeErrorCode::SessionMismatch => "claude_session_mismatch",
            ClaudeErrorCode::ProcessFailed => "claude_process_failed",
        }
    }
}

impl std::fmt::Display for ClaudeErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{code}: {detail}")]
pub struct ClaudeError {
    code: ClaudeErrorCode,
    detail: String,
}

impl ClaudeError {
    pub fn new(code: ClaudeErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    pub fn code(&self) -> ClaudeErrorCode {
        self.code
    }

    fn detail(&self) -> &str {
        &self.detail
    }
}

impl From<ClaudeError> for AppError {
    fn from(value: ClaudeError) -> Self {
        AppError::Message(value.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaudeLaunchTarget {
    Executable {
        executable: PathBuf,
    },
    NodeScript {
        node_executable: PathBuf,
        script: PathBuf,
    },
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ClaudeDetection {
    pub configured_path: String,
    pub resolved_path: String,
    pub launch_target: ClaudeLaunchTarget,
}

#[allow(dead_code)]
pub fn detect_cli(configured_path: Option<&Path>) -> Result<Vec<ClaudeDetection>, ClaudeError> {
    match configured_path {
        Some(path) if !path.as_os_str().is_empty() => {
            let home = std::env::var("USERPROFILE")
                .map(PathBuf::from)
                .unwrap_or_default();
            let cwd = std::env::current_dir().map_err(|error| {
                ClaudeError::new(
                    ClaudeErrorCode::ProcessFailed,
                    format!("failed to read current directory: {error}"),
                )
            })?;
            let normalized = normalize_configured(&cwd, &home, path);
            let path_env = std::env::var("PATH").unwrap_or_default();
            let candidates = resolve_entry(&normalized, &path_env)?;
            Ok(candidates
                .into_iter()
                .map(|(resolved, launch_target)| ClaudeDetection {
                    configured_path: normalized.to_string_lossy().to_string(),
                    resolved_path: resolved.to_string_lossy().to_string(),
                    launch_target,
                })
                .collect())
        }
        _ => detect_auto(),
    }
}

fn detect_auto() -> Result<Vec<ClaudeDetection>, ClaudeError> {
    let path_env = std::env::var("PATH").unwrap_or_default();
    let search_dirs = collect_search_dirs(&path_env);
    let candidates = find_auto_candidates(&search_dirs, &default_npm_global_dirs(), &path_env);
    detections_from_candidates(candidates)
}

fn detections_from_candidates(
    candidates: Vec<(PathBuf, ClaudeLaunchTarget)>,
) -> Result<Vec<ClaudeDetection>, ClaudeError> {
    if candidates.is_empty() {
        return Err(ClaudeError::new(
            ClaudeErrorCode::InvalidCliPath,
            "no Claude CLI found on PATH or in known npm locations",
        ));
    }
    Ok(candidates
        .into_iter()
        .map(|(resolved, launch_target)| ClaudeDetection {
            configured_path: String::new(),
            resolved_path: resolved.to_string_lossy().to_string(),
            launch_target,
        })
        .collect())
}

fn default_npm_global_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(appdata) = std::env::var("APPDATA") {
        dirs.push(PathBuf::from(appdata).join("npm"));
    }
    if let Ok(prefix) = std::env::var("npm_config_prefix") {
        dirs.push(PathBuf::from(prefix));
    }
    dirs
}

pub(crate) fn normalize_configured(base: &Path, home: &Path, path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if text == "~" {
        return home.to_path_buf();
    }
    if let Some(rest) = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        return home.join(rest);
    }
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

pub(crate) fn resolve_entry(
    path: &Path,
    path_env: &str,
) -> Result<Vec<(PathBuf, ClaudeLaunchTarget)>, ClaudeError> {
    if path.is_dir() {
        let mut candidates = Vec::new();
        let exe = path.join("claude.exe");
        if exe.is_file() {
            candidates.push((
                exe.clone(),
                ClaudeLaunchTarget::Executable { executable: exe },
            ));
        }
        let wrapper = path.join(wrapper_relative());
        if wrapper.is_file() {
            match build_node_target(&wrapper, path_env) {
                Ok(target) => candidates.push((wrapper, target)),
                Err(error) => {
                    if candidates.is_empty() {
                        return Err(error);
                    }
                }
            }
        }
        if candidates.is_empty() {
            return Err(invalid_path(path));
        }
        return Ok(candidates);
    }
    if !path.is_file() {
        return Err(invalid_path(path));
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());
    let candidate = match extension.as_deref() {
        Some("exe") if file_name.eq_ignore_ascii_case("claude.exe") => (
            path.to_path_buf(),
            ClaudeLaunchTarget::Executable {
                executable: path.to_path_buf(),
            },
        ),
        Some("cjs") if file_name.eq_ignore_ascii_case("cli-wrapper.cjs") => {
            (path.to_path_buf(), build_node_target(path, path_env)?)
        }
        Some("cmd") | Some("ps1") | None if file_stem_is_claude(path) => {
            let wrapper = resolve_shim(path).ok_or_else(|| invalid_path(path))?;
            (wrapper.clone(), build_node_target(&wrapper, path_env)?)
        }
        _ => return Err(invalid_path(path)),
    };
    Ok(vec![candidate])
}

fn file_stem_is_claude(path: &Path) -> bool {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.eq_ignore_ascii_case("claude"))
}

pub(crate) fn resolve_shim(shim: &Path) -> Option<PathBuf> {
    let dir = shim.parent()?;
    let structural = dir.join(wrapper_relative());
    if structural.is_file() {
        return Some(structural);
    }
    let content = std::fs::read_to_string(shim).ok()?;
    parse_shim_content_for_wrapper(dir, &content)
}

pub(crate) fn parse_shim_content_for_wrapper(shim_dir: &Path, content: &str) -> Option<PathBuf> {
    for line in content.lines() {
        let line = line.trim();
        if !line.contains("cli-wrapper.cjs") {
            continue;
        }
        for token in line.split_whitespace() {
            if !token.contains("cli-wrapper.cjs") {
                continue;
            }
            let cleaned = token.trim_matches(|c: char| c == '"' || c == '\'' || c == ';');
            let expanded = cleaned
                .replace("%~dp0\\", "")
                .replace("%~dp0/", "")
                .replace("%~dp0", "")
                .replace("$basedir\\", "")
                .replace("$basedir/", "")
                .replace("$basedir", "");
            if expanded.is_empty() {
                continue;
            }
            let candidate = if Path::new(&expanded).is_absolute() {
                lexical_normalize(Path::new(&expanded))
            } else {
                lexical_normalize(&shim_dir.join(&expanded))
            };
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

pub(crate) fn find_node_executable(hints: &[PathBuf], path_env: &str) -> Option<PathBuf> {
    for dir in hints.iter().chain(collect_search_dirs(path_env).iter()) {
        let node = dir.join("node.exe");
        if node.is_file() {
            return Some(node);
        }
    }
    None
}

fn node_hints(wrapper: &Path) -> Vec<PathBuf> {
    let mut hints = Vec::new();
    if let Some(parent) = wrapper.parent() {
        hints.push(parent.to_path_buf());
    }
    let mut current = wrapper.parent().map(|parent| parent.to_path_buf());
    while let Some(dir) = current {
        if dir.file_name().and_then(|name| name.to_str()) == Some("node_modules") {
            if let Some(root) = dir.parent() {
                hints.push(root.to_path_buf());
            }
            break;
        }
        current = dir.parent().map(|parent| parent.to_path_buf());
    }
    hints
}

pub(crate) fn collect_search_dirs(path_env: &str) -> Vec<PathBuf> {
    let separator = if cfg!(windows) { ';' } else { ':' };
    path_env
        .split(separator)
        .filter(|entry| !entry.is_empty())
        .map(PathBuf::from)
        .collect()
}

pub(crate) fn find_auto_candidates(
    search_dirs: &[PathBuf],
    npm_global_dirs: &[PathBuf],
    path_env: &str,
) -> Vec<(PathBuf, ClaudeLaunchTarget)> {
    let mut candidates = Vec::new();
    for dir in search_dirs {
        let exe = dir.join("claude.exe");
        if exe.is_file() {
            candidates.push((
                exe.clone(),
                ClaudeLaunchTarget::Executable { executable: exe },
            ));
        }
    }
    for dir in search_dirs {
        for shim_name in ["claude.cmd", "claude.ps1", "claude"] {
            let shim = dir.join(shim_name);
            if shim.is_file() {
                if let Some(wrapper) = resolve_shim(&shim) {
                    if let Ok(target) = build_node_target(&wrapper, path_env) {
                        candidates.push((wrapper, target));
                    }
                }
            }
        }
    }
    for dir in npm_global_dirs {
        let wrapper = dir.join(wrapper_relative());
        if wrapper.is_file() {
            if let Ok(target) = build_node_target(&wrapper, path_env) {
                candidates.push((wrapper, target));
            }
        }
    }
    candidates
}

fn build_node_target(wrapper: &Path, path_env: &str) -> Result<ClaudeLaunchTarget, ClaudeError> {
    let node = find_node_executable(&node_hints(wrapper), path_env).ok_or_else(|| {
        ClaudeError::new(
            ClaudeErrorCode::NodeNotFound,
            format!("node.exe is required to launch {}", wrapper.display()),
        )
    })?;
    Ok(ClaudeLaunchTarget::NodeScript {
        node_executable: node,
        script: wrapper.to_path_buf(),
    })
}

fn invalid_path(path: &Path) -> ClaudeError {
    ClaudeError::new(ClaudeErrorCode::InvalidCliPath, path.display().to_string())
}

fn wrapper_relative() -> PathBuf {
    PathBuf::from("node_modules")
        .join("@anthropic-ai")
        .join("claude-code")
        .join("cli-wrapper.cjs")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum TurnMode {
    NewSession,
    Resume,
}

#[derive(Debug, Clone)]
pub struct ClaudeTurnRequest<'a> {
    pub vault_root: &'a Path,
    pub session_id: &'a str,
    pub mode: TurnMode,
    pub prompt: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeTurnResult {
    pub answer: String,
    pub thinking: String,
    pub model: Option<String>,
    pub cli_version: Option<String>,
    pub used_fallback_resume: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaudeTurnError {
    Cancelled,
    SpawnFailed {
        message: String,
    },
    Io {
        message: String,
    },
    InitPersist {
        message: String,
    },
    Protocol {
        message: String,
    },
    SessionMismatch {
        message: String,
    },
    Process {
        stderr_tail: String,
        id_in_use: bool,
        no_conversation: bool,
        saw_init: bool,
    },
    CliError {
        code: ClaudeErrorCode,
        subtype: String,
        sanitized_result: Option<String>,
        exit_ok: bool,
    },
}

pub type InitializedCallback<'a> =
    Box<dyn Fn(&str, Option<&str>, Option<&str>) -> Result<(), String> + Send + Sync + 'a>;

pub struct TurnEvents {
    pub token: Box<dyn Fn(&str) + Send + Sync>,
    pub thinking: Box<dyn Fn(&str) + Send + Sync>,
    pub initialized: InitializedCallback<'static>,
}

impl Default for TurnEvents {
    fn default() -> Self {
        Self {
            token: Box::new(|_| {}),
            thinking: Box::new(|_| {}),
            initialized: Box::new(|_, _, _| Ok(())),
        }
    }
}

#[allow(dead_code)]
pub const PROBE_VERSION_TIMEOUT: Duration = Duration::from_secs(10);

const MAX_STDERR_BYTES: usize = 64 * 1024;
const MAX_PROBE_OUTPUT_BYTES: usize = 64 * 1024;
const STDOUT_CHANNEL_CAPACITY: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeFailure {
    Spawn { message: String },
    Timeout,
    Exited { stderr_tail: String },
    NoVersionOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    Version(String),
    Failed(ProbeFailure),
}

#[allow(dead_code)]
pub type CancelToken = std::sync::Arc<std::sync::atomic::AtomicBool>;

#[allow(dead_code)]
pub fn never_cancel() -> CancelToken {
    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))
}

fn version_args() -> Vec<String> {
    vec!["--version".to_string()]
}

fn turn_args(mode: TurnMode, session_id: &str) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--include-partial-messages".to_string(),
    ];
    match mode {
        TurnMode::NewSession => {
            args.push("--session-id".to_string());
            args.push(session_id.to_string());
        }
        TurnMode::Resume => {
            args.push("--resume".to_string());
            args.push(session_id.to_string());
        }
    }
    args
}

fn spawn_command(detection: &ClaudeDetection, args: &[String]) -> tokio::process::Command {
    let mut command = match &detection.launch_target {
        ClaudeLaunchTarget::Executable { executable } => {
            let mut command = tokio::process::Command::new(executable);
            command.args(args);
            command
        }
        ClaudeLaunchTarget::NodeScript {
            node_executable,
            script,
        } => {
            let mut command = tokio::process::Command::new(node_executable);
            command.arg(script).args(args);
            command
        }
    };
    command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    command
}

fn strip_ansi(text: &str) -> String {
    let mut cleaned = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            while let Some(&next) = chars.peek() {
                chars.next();
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            cleaned.push(c);
        }
    }
    cleaned
}

pub fn is_session_id_in_use_error(stderr: &str) -> bool {
    let cleaned = strip_ansi(stderr);
    cleaned.contains("Session ID") && cleaned.contains("is already in use")
}

pub fn is_no_conversation_found_error(stderr: &str) -> bool {
    let cleaned = strip_ansi(stderr);
    cleaned.contains("No conversation found")
}

enum Keep {
    Head,
    Tail,
}

async fn read_capped<R: tokio::io::AsyncRead + Unpin>(
    mut stream: Option<R>,
    cap: usize,
    keep: Keep,
) -> String {
    use tokio::io::AsyncReadExt;
    let mut buffer: Vec<u8> = Vec::new();
    if let Some(stream) = stream.as_mut() {
        let mut chunk = [0u8; 8192];
        loop {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    buffer.extend_from_slice(&chunk[..n]);
                    if buffer.len() > cap {
                        match keep {
                            Keep::Head => buffer.truncate(cap),
                            Keep::Tail => {
                                let excess = buffer.len() - cap;
                                buffer.drain(..excess);
                            }
                        }
                    }
                }
            }
        }
    }
    String::from_utf8_lossy(&buffer).to_string()
}

#[cfg(windows)]
fn kill_process_tree(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        let _ = std::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

#[cfg(not(windows))]
fn kill_process_tree(child: &mut tokio::process::Child) {
    let _ = child.start_kill();
}

async fn kill_and_reap(child: &mut tokio::process::Child) {
    kill_process_tree(child);
    let _ = child.wait().await;
}

async fn cancel_notify(cancel: &CancelToken) {
    while !cancel.load(std::sync::atomic::Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[allow(dead_code)]
pub async fn probe_version(detection: &ClaudeDetection, timeout: Duration) -> ProbeOutcome {
    let mut command = spawn_command(detection, &version_args());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return ProbeOutcome::Failed(ProbeFailure::Spawn {
                message: error.to_string(),
            })
        }
    };
    drop(child.stdin.take());
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_task = tokio::spawn(read_capped(stdout, MAX_PROBE_OUTPUT_BYTES, Keep::Head));
    let stderr_task = tokio::spawn(read_capped(stderr, MAX_STDERR_BYTES, Keep::Tail));
    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(status) => status,
        Err(_) => {
            kill_and_reap(&mut child).await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            return ProbeOutcome::Failed(ProbeFailure::Timeout);
        }
    };
    let stdout_text = stdout_task.await.unwrap_or_default();
    let stderr_tail = stderr_task.await.unwrap_or_default();
    if !matches!(&status, Ok(status) if status.success()) {
        return ProbeOutcome::Failed(ProbeFailure::Exited { stderr_tail });
    }
    let first_token = stdout_text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .and_then(|line| line.split_whitespace().next())
        .unwrap_or_default()
        .to_string();
    if first_token.is_empty() {
        return ProbeOutcome::Failed(ProbeFailure::NoVersionOutput);
    }
    ProbeOutcome::Version(first_token)
}

#[allow(dead_code)]
pub async fn run_turn(
    detection: &ClaudeDetection,
    request: ClaudeTurnRequest<'_>,
    events: &TurnEvents,
    cancel: &CancelToken,
) -> Result<ClaudeTurnResult, ClaudeTurnError> {
    let primary = execute_single_turn(detection, &request, events, cancel).await;
    match primary {
        Ok(result) => Ok(result),
        Err(ClaudeTurnError::Process {
            id_in_use: true,
            saw_init: false,
            ..
        }) if request.mode == TurnMode::NewSession => {
            let fallback_request = ClaudeTurnRequest {
                mode: TurnMode::Resume,
                ..request.clone()
            };
            match execute_single_turn(detection, &fallback_request, events, cancel).await {
                Ok(mut result) => {
                    result.used_fallback_resume = true;
                    Ok(result)
                }
                Err(ClaudeTurnError::Process {
                    no_conversation: true,
                    ..
                }) => Err(ClaudeTurnError::Protocol {
                    message: "session id is in use but not resumable".to_string(),
                }),
                Err(other) => Err(other),
            }
        }
        Err(other) => Err(other),
    }
}

async fn execute_single_turn(
    detection: &ClaudeDetection,
    request: &ClaudeTurnRequest<'_>,
    events: &TurnEvents,
    cancel: &CancelToken,
) -> Result<ClaudeTurnResult, ClaudeTurnError> {
    let args = turn_args(request.mode, request.session_id);
    let mut command = spawn_command(detection, &args);
    command.current_dir(request.vault_root);
    let mut child = command
        .spawn()
        .map_err(|error| ClaudeTurnError::SpawnFailed {
            message: error.to_string(),
        })?;

    if let Some(stdin) = child.stdin.take() {
        let mut stdin = stdin;
        let write_fut = async {
            stdin.write_all(request.prompt.as_bytes()).await?;
            stdin.shutdown().await
        };
        tokio::pin!(write_fut);
        let outcome = tokio::select! {
            biased;
            _ = cancel_notify(cancel) => None,
            result = &mut write_fut => Some(result),
        };
        match outcome {
            None => {
                kill_and_reap(&mut child).await;
                return Err(ClaudeTurnError::Cancelled);
            }
            Some(Err(error)) => {
                kill_and_reap(&mut child).await;
                return Err(ClaudeTurnError::Io {
                    message: error.to_string(),
                });
            }
            Some(Ok(())) => {}
        }
    }

    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            kill_and_reap(&mut child).await;
            return Err(ClaudeTurnError::Io {
                message: "no stdout pipe".to_string(),
            });
        }
    };
    let stderr = child.stderr.take();

    let (line_tx, mut line_rx) = tokio::sync::mpsc::channel::<String>(STDOUT_CHANNEL_CAPACITY);
    let stdout_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            if line_tx.send(line).await.is_err() {
                break;
            }
        }
    });
    let stderr_task = tokio::spawn(read_capped(stderr, MAX_STDERR_BYTES, Keep::Tail));

    let mut parser = StreamParser::new(request.session_id);
    let mut abort: Option<ClaudeTurnError> = None;
    loop {
        tokio::select! {
            biased;
            _ = cancel_notify(cancel) => {
                abort = Some(ClaudeTurnError::Cancelled);
                break;
            }
            maybe_line = line_rx.recv() => {
                match maybe_line {
                    Some(line) => {
                        match parser.ingest_line(&line) {
                            Ok(Some(ParserEvent::Token(text))) => (events.token)(&text),
                            Ok(Some(ParserEvent::Thinking(text))) => (events.thinking)(&text),
                            Ok(Some(ParserEvent::Initialized { session_id, model, cli_version })) => {
                                if let Err(message) = (events.initialized)(
                                    &session_id,
                                    model.as_deref(),
                                    cli_version.as_deref(),
                                ) {
                                    abort = Some(ClaudeTurnError::InitPersist { message });
                                    break;
                                }
                            }
                            Ok(None) => {}
                            Err(error) => {
                                abort = Some(turn_error_from_parser(error));
                                break;
                            }
                        }
                    }
                    None => break,
                }
            }
        }
    }

    if let Some(error) = abort {
        kill_and_reap(&mut child).await;
        drop(line_rx);
        let _ = stdout_task.await;
        let _ = stderr_task.await;
        return Err(error);
    }

    let status = tokio::select! {
        biased;
        _ = cancel_notify(cancel) => {
            kill_and_reap(&mut child).await;
            drop(line_rx);
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            return Err(ClaudeTurnError::Cancelled);
        }
        status = child.wait() => status,
    };
    let exit_ok = matches!(&status, Ok(status) if status.success());
    let stderr_tail = stderr_task.await.unwrap_or_default();
    let _ = stdout_task.await;

    match parser.finish(exit_ok) {
        Ok(TurnOutcome::Success {
            answer,
            thinking,
            model,
            cli_version,
            ..
        }) => Ok(ClaudeTurnResult {
            answer,
            thinking,
            model,
            cli_version,
            used_fallback_resume: false,
        }),
        Ok(TurnOutcome::Failed {
            code: ClaudeErrorCode::ProcessFailed,
            subtype: None,
            ..
        }) => {
            let id_in_use = is_session_id_in_use_error(&stderr_tail);
            let no_conversation = is_no_conversation_found_error(&stderr_tail);
            Err(ClaudeTurnError::Process {
                stderr_tail,
                id_in_use,
                no_conversation,
                saw_init: parser.saw_valid_init(),
            })
        }
        Ok(TurnOutcome::Failed {
            code,
            subtype,
            sanitized_result,
            exit_ok,
        }) => Err(ClaudeTurnError::CliError {
            code,
            subtype: subtype.unwrap_or_default(),
            sanitized_result,
            exit_ok,
        }),
        Err(error) => Err(turn_error_from_parser(error)),
    }
}

fn turn_error_from_parser(error: ClaudeError) -> ClaudeTurnError {
    match error.code() {
        ClaudeErrorCode::SessionMismatch => ClaudeTurnError::SessionMismatch {
            message: error.detail().to_string(),
        },
        _ => ClaudeTurnError::Protocol {
            message: error.detail().to_string(),
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParserEvent {
    Token(String),
    Thinking(String),
    Initialized {
        session_id: String,
        model: Option<String>,
        cli_version: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome {
    Success {
        answer: String,
        thinking: String,
        cli_session_id: String,
        model: Option<String>,
        cli_version: Option<String>,
    },
    Failed {
        code: ClaudeErrorCode,
        subtype: Option<String>,
        sanitized_result: Option<String>,
        exit_ok: bool,
    },
}

pub struct StreamParser {
    expected_session_id: String,
    state: ParserState,
}

#[derive(Default)]
struct ParserState {
    saw_valid_init: bool,
    model: Option<String>,
    cli_version: Option<String>,
    streamed_text: String,
    streamed_thinking: String,
    assistant_text: Option<String>,
    assistant_thinking: Option<String>,
    result: Option<ParsedResultMessage>,
}

#[derive(Debug, Clone)]
struct ParsedResultMessage {
    subtype: String,
    session_id: String,
    is_error: bool,
    text: Option<String>,
}

fn preview(text: &str) -> String {
    truncate_chars(text, 80)
}

fn sanitize_result_text(text: &str) -> String {
    truncate_chars(text, 500)
}

fn truncate_chars(text: &str, limit: usize) -> String {
    let mut shown: String = text.chars().take(limit).collect();
    if shown.len() < text.len() {
        shown.push('…');
    }
    shown
}

fn pick_final(streamed: &str, candidates: &[Option<&String>]) -> String {
    for candidate in candidates.iter().flatten() {
        if !candidate.is_empty() {
            if !streamed.is_empty() && !candidate.starts_with(streamed) {
                crate::nest_debug!(
                    "claude_cli",
                    "final candidate conflicts with streamed text; using the final candidate"
                );
            }
            return (*candidate).clone();
        }
    }
    streamed.to_string()
}

#[allow(dead_code)]
impl StreamParser {
    pub fn new(expected_session_id: &str) -> Self {
        Self {
            expected_session_id: expected_session_id.to_string(),
            state: ParserState::default(),
        }
    }

    pub fn saw_valid_init(&self) -> bool {
        self.state.saw_valid_init
    }

    pub fn ingest_line(&mut self, line: &str) -> Result<Option<ParserEvent>, ClaudeError> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        let value: serde_json::Value = serde_json::from_str(trimmed).map_err(|_| {
            ClaudeError::new(
                ClaudeErrorCode::Protocol,
                format!("non-JSON output line: {}", preview(trimmed)),
            )
        })?;
        let kind = value
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        match kind {
            "system" => self.handle_system(&value),
            "stream_event" => self.handle_stream_event(&value),
            "assistant" => {
                self.handle_assistant(&value);
                Ok(None)
            }
            "result" => {
                self.handle_result(&value)?;
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn handle_system(
        &mut self,
        value: &serde_json::Value,
    ) -> Result<Option<ParserEvent>, ClaudeError> {
        if value.get("subtype").and_then(|value| value.as_str()) != Some("init") {
            return Ok(None);
        }
        let session_id = value
            .get("session_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        if session_id != self.expected_session_id {
            return Err(ClaudeError::new(
                ClaudeErrorCode::SessionMismatch,
                format!(
                    "init session {session_id} does not match {}",
                    self.expected_session_id
                ),
            ));
        }
        let model = value
            .get("model")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let cli_version = value
            .get("claude_code_version")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        self.state.saw_valid_init = true;
        self.state.model = model.clone();
        self.state.cli_version = cli_version.clone();
        Ok(Some(ParserEvent::Initialized {
            session_id,
            model,
            cli_version,
        }))
    }

    fn handle_stream_event(
        &mut self,
        value: &serde_json::Value,
    ) -> Result<Option<ParserEvent>, ClaudeError> {
        let Some(event) = value.get("event") else {
            return Ok(None);
        };
        if event.get("type").and_then(|value| value.as_str()) != Some("content_block_delta") {
            return Ok(None);
        }
        let Some(delta) = event.get("delta") else {
            return Ok(None);
        };
        match delta.get("type").and_then(|value| value.as_str()) {
            Some("text_delta") => {
                let text = delta
                    .get("text")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                if text.is_empty() {
                    return Ok(None);
                }
                self.state.streamed_text.push_str(text);
                Ok(Some(ParserEvent::Token(text.to_string())))
            }
            Some("thinking_delta") => {
                let thinking = delta
                    .get("thinking")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                if thinking.is_empty() {
                    return Ok(None);
                }
                self.state.streamed_thinking.push_str(thinking);
                Ok(Some(ParserEvent::Thinking(thinking.to_string())))
            }
            _ => Ok(None),
        }
    }

    fn handle_assistant(&mut self, value: &serde_json::Value) {
        let Some(content) = value
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(|content| content.as_array())
        else {
            return;
        };
        let mut text = String::new();
        let mut thinking = String::new();
        for block in content {
            match block.get("type").and_then(|value| value.as_str()) {
                Some("text") => {
                    if let Some(chunk) = block.get("text").and_then(|value| value.as_str()) {
                        text.push_str(chunk);
                    }
                }
                Some("thinking") => {
                    if let Some(chunk) = block.get("thinking").and_then(|value| value.as_str()) {
                        thinking.push_str(chunk);
                    }
                }
                _ => {}
            }
        }
        if !text.is_empty() {
            self.state.assistant_text = Some(text);
        }
        if !thinking.is_empty() {
            self.state.assistant_thinking = Some(thinking);
        }
    }

    fn handle_result(&mut self, value: &serde_json::Value) -> Result<(), ClaudeError> {
        let session_id = value
            .get("session_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        if session_id.is_empty() {
            return Err(ClaudeError::new(
                ClaudeErrorCode::Protocol,
                "result message without session_id",
            ));
        }
        if session_id != self.expected_session_id {
            return Err(ClaudeError::new(
                ClaudeErrorCode::SessionMismatch,
                format!(
                    "result session {session_id} does not match {}",
                    self.expected_session_id
                ),
            ));
        }
        self.state.result = Some(ParsedResultMessage {
            subtype: value
                .get("subtype")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
            session_id,
            is_error: value
                .get("is_error")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            text: value
                .get("result")
                .and_then(|value| value.as_str())
                .map(str::to_string),
        });
        Ok(())
    }

    pub fn finish(&mut self, exit_ok: bool) -> Result<TurnOutcome, ClaudeError> {
        let Some(result) = self.state.result.clone() else {
            if exit_ok {
                return Err(ClaudeError::new(
                    ClaudeErrorCode::Protocol,
                    "CLI exited without a result message",
                ));
            }
            return Ok(TurnOutcome::Failed {
                code: ClaudeErrorCode::ProcessFailed,
                subtype: None,
                sanitized_result: None,
                exit_ok,
            });
        };
        if result.is_error || result.subtype != "success" {
            return Ok(TurnOutcome::Failed {
                code: ClaudeErrorCode::Protocol,
                subtype: Some(result.subtype),
                sanitized_result: result.text.as_deref().map(sanitize_result_text),
                exit_ok,
            });
        }
        if !exit_ok {
            return Ok(TurnOutcome::Failed {
                code: ClaudeErrorCode::ProcessFailed,
                subtype: Some(result.subtype),
                sanitized_result: result.text.as_deref().map(sanitize_result_text),
                exit_ok,
            });
        }
        if !self.state.saw_valid_init {
            return Err(ClaudeError::new(
                ClaudeErrorCode::Protocol,
                "result without a matching system/init",
            ));
        }
        let answer = pick_final(
            &self.state.streamed_text,
            &[result.text.as_ref(), self.state.assistant_text.as_ref()],
        );
        let thinking = pick_final(
            &self.state.streamed_thinking,
            &[self.state.assistant_thinking.as_ref()],
        );
        Ok(TurnOutcome::Success {
            answer,
            thinking,
            cli_session_id: result.session_id,
            model: self.state.model.clone(),
            cli_version: self.state.cli_version.clone(),
        })
    }
}

#[cfg(test)]
mod resolver_tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Fixture {
            let root = std::env::temp_dir().join(format!(
                "nest-claude-cli-test-{}-{}",
                name,
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&root).unwrap();
            Fixture { root }
        }

        fn file(&self, rel: &str) -> PathBuf {
            self.root.join(rel)
        }

        fn touch(&self, rel: &str) -> PathBuf {
            let path = self.file(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, b"").unwrap();
            path
        }

        fn npm_layout(&self, dir: &str) -> PathBuf {
            let base = self.root.join(dir);
            let shim_dir = base.join("bin");
            self.touch(&format!(
                "{dir}/bin/node_modules/@anthropic-ai/claude-code/cli-wrapper.cjs"
            ));
            self.touch(&format!("{dir}/bin/node.exe"));
            shim_dir
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn explicit_exe_file_launches_directly() {
        let fx = Fixture::new("explicit-exe");
        let exe = fx.touch("tools/claude.exe");
        let detections = detect_cli(Some(&exe)).unwrap();
        assert_eq!(detections.len(), 1);
        assert_eq!(
            detections[0].launch_target,
            ClaudeLaunchTarget::Executable {
                executable: exe.clone()
            }
        );
        assert_eq!(
            detections[0].resolved_path,
            exe.to_string_lossy().to_string()
        );
    }

    #[test]
    fn explicit_wrapper_script_launches_via_node() {
        let fx = Fixture::new("explicit-wrapper");
        let wrapper = fx.touch("npm/node_modules/@anthropic-ai/claude-code/cli-wrapper.cjs");
        let node = fx.touch("npm/node.exe");
        let detections = detect_cli(Some(&wrapper)).unwrap();
        assert_eq!(detections.len(), 1);
        match &detections[0].launch_target {
            ClaudeLaunchTarget::NodeScript {
                node_executable,
                script,
            } => {
                assert_eq!(*script, wrapper);
                assert_eq!(*node_executable, node);
            }
            other => panic!("expected NodeScript, got {other:?}"),
        }
    }

    #[test]
    fn cmd_shim_resolves_to_wrapper() {
        let fx = Fixture::new("cmd-shim");
        let shim_dir = fx.npm_layout("install");
        let shim = shim_dir.join("claude.cmd");
        std::fs::write(
            &shim,
            b"@echo off\r\nnode \"%~dp0\\node_modules\\@anthropic-ai\\claude-code\\cli-wrapper.cjs\" %*\r\n",
        )
        .unwrap();
        let detections = detect_cli(Some(&shim)).unwrap();
        assert_eq!(detections.len(), 1);
        assert!(matches!(
            &detections[0].launch_target,
            ClaudeLaunchTarget::NodeScript { script, .. } if script.ends_with("cli-wrapper.cjs")
        ));
    }

    #[test]
    fn ps1_shim_resolves_to_wrapper() {
        let fx = Fixture::new("ps1-shim");
        let shim_dir = fx.npm_layout("install");
        let shim = shim_dir.join("claude.ps1");
        std::fs::write(
            &shim,
            b"$basedir = Split-Path $MyInvocation.MyCommand.Definition -Parent\n",
        )
        .unwrap();
        let detections = detect_cli(Some(&shim)).unwrap();
        assert!(matches!(
            &detections[0].launch_target,
            ClaudeLaunchTarget::NodeScript { .. }
        ));
    }

    #[test]
    fn extensionless_shim_resolves_to_wrapper() {
        let fx = Fixture::new("extless-shim");
        let shim_dir = fx.npm_layout("install");
        let shim = shim_dir.join("claude");
        std::fs::write(&shim, b"#!/bin/sh\n").unwrap();
        let detections = detect_cli(Some(&shim)).unwrap();
        assert!(matches!(
            &detections[0].launch_target,
            ClaudeLaunchTarget::NodeScript { .. }
        ));
    }

    #[test]
    fn shim_content_fallback_when_layout_is_unusual() {
        let fx = Fixture::new("shim-content");
        let wrapper = fx.touch("elsewhere/@anthropic-ai/claude-code/cli-wrapper.cjs");
        let shim = fx.file("bin/claude.cmd");
        std::fs::create_dir_all(shim.parent().unwrap()).unwrap();
        let content =
            "@node \"%~dp0\\..\\elsewhere\\@anthropic-ai\\claude-code\\cli-wrapper.cjs\" %*\r\n";
        std::fs::write(&shim, content).unwrap();
        let parsed = parse_shim_content_for_wrapper(
            shim.parent().unwrap(),
            &std::fs::read_to_string(&shim).unwrap(),
        )
        .unwrap();
        assert_eq!(parsed, wrapper);
    }

    #[test]
    fn directory_candidates_prefer_native_exe_then_wrapper() {
        let fx = Fixture::new("dir-both");
        let dir = fx.root.join("dir");
        let exe = fx.touch("dir/claude.exe");
        let wrapper = fx.touch("dir/node_modules/@anthropic-ai/claude-code/cli-wrapper.cjs");
        fx.touch("dir/node.exe");
        let detections = detect_cli(Some(&dir)).unwrap();
        assert_eq!(detections.len(), 2);
        assert_eq!(
            detections[0].launch_target,
            ClaudeLaunchTarget::Executable { executable: exe }
        );
        assert!(matches!(
            &detections[1].launch_target,
            ClaudeLaunchTarget::NodeScript { script, .. } if *script == wrapper
        ));
    }

    #[test]
    fn directory_falls_back_to_wrapper_layout() {
        let fx = Fixture::new("dir-wrapper");
        let dir = fx.root.join("dir");
        fx.touch("dir/node_modules/@anthropic-ai/claude-code/cli-wrapper.cjs");
        fx.touch("dir/node.exe");
        let detections = detect_cli(Some(&dir)).unwrap();
        assert_eq!(detections.len(), 1);
        assert!(matches!(
            &detections[0].launch_target,
            ClaudeLaunchTarget::NodeScript { .. }
        ));
    }

    #[test]
    fn directory_without_candidates_is_invalid() {
        let fx = Fixture::new("dir-empty");
        let dir = fx.root.join("dir");
        std::fs::create_dir_all(&dir).unwrap();
        let err = detect_cli(Some(&dir)).unwrap_err();
        assert_eq!(err.code(), ClaudeErrorCode::InvalidCliPath);
    }

    #[test]
    fn missing_file_is_invalid() {
        let fx = Fixture::new("missing");
        let err = detect_cli(Some(&fx.file("nope/claude.exe"))).unwrap_err();
        assert_eq!(err.code(), ClaudeErrorCode::InvalidCliPath);
    }

    #[test]
    fn unsupported_file_is_invalid() {
        let fx = Fixture::new("unsupported");
        let txt = fx.touch("claude.txt");
        let err = detect_cli(Some(&txt)).unwrap_err();
        assert_eq!(err.code(), ClaudeErrorCode::InvalidCliPath);
    }

    #[test]
    fn arbitrary_exe_files_are_rejected() {
        let fx = Fixture::new("evil-exe");
        let exe = fx.touch("tools/evil.exe");
        let err = detect_cli(Some(&exe)).unwrap_err();
        assert_eq!(err.code(), ClaudeErrorCode::InvalidCliPath);
    }

    #[test]
    fn arbitrary_scripts_are_rejected() {
        let fx = Fixture::new("evil-cjs");
        let script = fx.touch("tools/other.cjs");
        let err = detect_cli(Some(&script)).unwrap_err();
        assert_eq!(err.code(), ClaudeErrorCode::InvalidCliPath);
    }

    #[test]
    fn non_claude_shims_are_rejected() {
        let fx = Fixture::new("evil-cmd");
        let shim = fx.touch("tools/not-claude.cmd");
        let err = detect_cli(Some(&shim)).unwrap_err();
        assert_eq!(err.code(), ClaudeErrorCode::InvalidCliPath);
    }

    #[test]
    fn node_missing_fails_the_wrapper_resolution() {
        let fx = Fixture::new("no-node");
        let wrapper = fx.touch("npm/node_modules/@anthropic-ai/claude-code/cli-wrapper.cjs");
        let err = resolve_entry(&wrapper, "").unwrap_err();
        assert_eq!(err.code(), ClaudeErrorCode::NodeNotFound);
        fx.touch("npm/node.exe");
        let candidates = resolve_entry(&wrapper, "").unwrap();
        assert!(matches!(
            &candidates[0].1,
            ClaudeLaunchTarget::NodeScript { .. }
        ));
    }

    #[test]
    fn paths_with_spaces_resolve() {
        let fx = Fixture::new("spaces");
        let exe = fx.touch("my tools/claude.exe");
        let detections = detect_cli(Some(&exe)).unwrap();
        assert_eq!(
            detections[0].launch_target,
            ClaudeLaunchTarget::Executable { executable: exe }
        );
    }

    #[test]
    fn relative_paths_normalize_against_base() {
        let fx = Fixture::new("relative");
        let exe = fx.touch("bin/claude.exe");
        let normalized = normalize_configured(&fx.root, &fx.root, Path::new("bin/claude.exe"));
        assert_eq!(normalized, exe);
    }

    #[test]
    fn home_tilde_expands() {
        let fx = Fixture::new("tilde");
        let home = fx.root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let exe = fx.touch("home/tools/claude.exe");
        let normalized = normalize_configured(&fx.root, &home, Path::new("~/tools/claude.exe"));
        assert_eq!(normalized, exe);
    }

    #[test]
    fn absolute_paths_pass_through() {
        let fx = Fixture::new("absolute");
        let exe = fx.touch("bin/claude.exe");
        let normalized = normalize_configured(&fx.root, &fx.root, &exe);
        assert_eq!(normalized, exe);
    }

    #[test]
    fn path_env_splits_into_directories() {
        let dirs = collect_search_dirs("C:\\a\\bin;D:\\tools;;C:\\b");
        assert_eq!(dirs.len(), 3);
        assert_eq!(dirs[0], PathBuf::from("C:\\a\\bin"));
        assert_eq!(dirs[2], PathBuf::from("C:\\b"));
    }

    #[test]
    fn auto_candidates_prefer_path_exe_over_shims() {
        let fx = Fixture::new("auto-order");
        let dir_a = fx.root.join("a");
        fx.touch("a/claude.cmd");
        fx.touch("a/node_modules/@anthropic-ai/claude-code/cli-wrapper.cjs");
        fx.touch("a/node.exe");
        let dir_b = fx.root.join("b");
        let exe = fx.touch("b/claude.exe");

        let candidates = find_auto_candidates(&[dir_a, dir_b], &[], "");
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].0, exe);
        assert!(matches!(
            &candidates[0].1,
            ClaudeLaunchTarget::Executable { .. }
        ));
    }

    #[test]
    fn auto_candidates_use_path_shim_when_no_exe() {
        let fx = Fixture::new("auto-shim");
        let dir_a = fx.root.join("a");
        fx.touch("a/claude.cmd");
        let wrapper = fx.touch("a/node_modules/@anthropic-ai/claude-code/cli-wrapper.cjs");
        fx.touch("a/node.exe");
        let candidates = find_auto_candidates(&[dir_a], &[], "");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, wrapper);
        assert!(matches!(
            &candidates[0].1,
            ClaudeLaunchTarget::NodeScript { .. }
        ));
    }

    #[test]
    fn auto_candidates_include_npm_global_wrapper() {
        let fx = Fixture::new("auto-global");
        let global = fx.root.join("npm-global");
        let wrapper = fx.touch("npm-global/node_modules/@anthropic-ai/claude-code/cli-wrapper.cjs");
        let node = fx.touch("npm-global/node.exe");
        let candidates = find_auto_candidates(&[], &[global], "");
        assert_eq!(candidates.len(), 1);
        match &candidates[0].1 {
            ClaudeLaunchTarget::NodeScript {
                script,
                node_executable,
            } => {
                assert_eq!(*script, wrapper);
                assert_eq!(*node_executable, node);
            }
            other => panic!("expected NodeScript, got {other:?}"),
        }
    }

    #[test]
    fn broken_install_does_not_shadow_later_candidates() {
        let fx = Fixture::new("auto-broken");
        let dir_a = fx.root.join("a");
        fx.touch("a/claude.cmd");
        fx.touch("a/node_modules/@anthropic-ai/claude-code/cli-wrapper.cjs");
        let dir_b = fx.root.join("b");
        let exe = fx.touch("b/claude.exe");

        let candidates = find_auto_candidates(&[dir_a, dir_b], &[], "");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, exe);
    }

    #[test]
    fn auto_detection_failure_is_invalid_path() {
        let err = detections_from_candidates(Vec::new()).unwrap_err();
        assert_eq!(err.code(), ClaudeErrorCode::InvalidCliPath);
    }
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    const SESSION: &str = "11111111-2222-4333-8444-555555555555";

    fn init_line() -> String {
        format!(
            r#"{{"type":"system","subtype":"init","session_id":"{SESSION}","model":"glm-5.3[1m]","claude_code_version":"2.1.238"}}"#
        )
    }

    fn feed_init(parser: &mut StreamParser) {
        match parser.ingest_line(&init_line()).unwrap() {
            Some(ParserEvent::Initialized { .. }) => {}
            other => panic!("expected Initialized event, got {other:?}"),
        }
    }

    fn text_delta(text: &str) -> String {
        format!(
            r#"{{"type":"stream_event","event":{{"type":"content_block_delta","index":1,"delta":{{"type":"text_delta","text":{}}}}},"session_id":"{SESSION}"}}"#,
            serde_json::json!(text)
        )
    }

    fn thinking_delta(text: &str) -> String {
        format!(
            r#"{{"type":"stream_event","event":{{"type":"content_block_delta","index":0,"delta":{{"type":"thinking_delta","thinking":{}}}}},"session_id":"{SESSION}"}}"#,
            serde_json::json!(text)
        )
    }

    fn assistant_line(text: &str) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":{}}}]}},"session_id":"{SESSION}"}}"#,
            serde_json::json!(text)
        )
    }

    fn result_line(subtype: &str, text: &str) -> String {
        format!(
            r#"{{"type":"result","subtype":"{subtype}","session_id":"{SESSION}","result":{}}}"#,
            serde_json::json!(text)
        )
    }

    #[test]
    fn init_emits_initialized_event_with_metadata() {
        let mut parser = StreamParser::new(SESSION);
        assert_eq!(
            parser.ingest_line(&init_line()).unwrap(),
            Some(ParserEvent::Initialized {
                session_id: SESSION.to_string(),
                model: Some("glm-5.3[1m]".to_string()),
                cli_version: Some("2.1.238".to_string()),
            })
        );
        assert!(parser.saw_valid_init());
        let err = parser.finish(true).unwrap_err();
        assert_eq!(err.code(), ClaudeErrorCode::Protocol);
    }

    #[test]
    fn init_session_mismatch_is_an_error() {
        let mut parser = StreamParser::new(SESSION);
        let foreign = r#"{"type":"system","subtype":"init","session_id":"88888888-8888-4888-8888-888888888888","model":"m"}"#;
        let err = parser.ingest_line(foreign).unwrap_err();
        assert_eq!(err.code(), ClaudeErrorCode::SessionMismatch);
    }

    #[test]
    fn text_deltas_emit_tokens() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        assert_eq!(
            parser.ingest_line(&text_delta("hel")).unwrap(),
            Some(ParserEvent::Token("hel".into()))
        );
        assert_eq!(
            parser.ingest_line(&text_delta("lo")).unwrap(),
            Some(ParserEvent::Token("lo".into()))
        );
    }

    #[test]
    fn thinking_deltas_emit_thinking_events() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        assert_eq!(
            parser
                .ingest_line(&thinking_delta("Let me think."))
                .unwrap(),
            Some(ParserEvent::Thinking("Let me think.".into()))
        );
    }

    #[test]
    fn partial_and_final_do_not_duplicate() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        parser.ingest_line(&text_delta("hello")).unwrap();
        parser.ingest_line(&text_delta(" world")).unwrap();
        assert_eq!(
            parser.ingest_line(&assistant_line("hello world")).unwrap(),
            None
        );
        assert_eq!(
            parser
                .ingest_line(&result_line("success", "hello world"))
                .unwrap(),
            None
        );
        let TurnOutcome::Success { answer, .. } = parser.finish(true).unwrap() else {
            panic!("expected success");
        };
        assert_eq!(answer, "hello world");
    }

    #[test]
    fn final_only_streams_once_at_finish() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        assert_eq!(
            parser.ingest_line(&assistant_line("hello world")).unwrap(),
            None
        );
        assert_eq!(
            parser
                .ingest_line(&result_line("success", "hello world"))
                .unwrap(),
            None
        );
        let TurnOutcome::Success { answer, .. } = parser.finish(true).unwrap() else {
            panic!("expected success");
        };
        assert_eq!(answer, "hello world");
    }

    #[test]
    fn result_takes_priority_over_longer_assistant() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        parser.ingest_line(&text_delta("hel")).unwrap();
        parser
            .ingest_line(&assistant_line("hello world and more"))
            .unwrap();
        parser
            .ingest_line(&result_line("success", "hello"))
            .unwrap();
        let TurnOutcome::Success { answer, .. } = parser.finish(true).unwrap() else {
            panic!("expected success");
        };
        assert_eq!(answer, "hello");
    }

    #[test]
    fn result_error_subtype_fails_with_structured_info() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        let error_result = format!(
            r#"{{"type":"result","subtype":"error_during_execution","session_id":"{SESSION}","is_error":true,"result":"boom"}}"#
        );
        parser.ingest_line(&error_result).unwrap();
        let TurnOutcome::Failed {
            code,
            subtype,
            sanitized_result,
            exit_ok,
        } = parser.finish(true).unwrap()
        else {
            panic!("expected failure");
        };
        assert_eq!(code, ClaudeErrorCode::Protocol);
        assert_eq!(subtype.as_deref(), Some("error_during_execution"));
        assert_eq!(sanitized_result.as_deref(), Some("boom"));
        assert!(exit_ok);
    }

    #[test]
    fn error_result_survives_non_zero_exit() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        let error_result = format!(
            r#"{{"type":"result","subtype":"error_during_execution","session_id":"{SESSION}","is_error":true,"result":"auth expired"}}"#
        );
        parser.ingest_line(&error_result).unwrap();
        let TurnOutcome::Failed {
            code,
            subtype,
            sanitized_result,
            exit_ok,
        } = parser.finish(false).unwrap()
        else {
            panic!("expected failure");
        };
        assert_eq!(code, ClaudeErrorCode::Protocol);
        assert_eq!(subtype.as_deref(), Some("error_during_execution"));
        assert_eq!(sanitized_result.as_deref(), Some("auth expired"));
        assert!(!exit_ok);
    }

    #[test]
    fn sanitized_result_truncates_long_error_text() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        let long_text = "x".repeat(600);
        let error_result = format!(
            r#"{{"type":"result","subtype":"error_max_turns","session_id":"{SESSION}","is_error":true,"result":"{}"}}"#,
            long_text
        );
        parser.ingest_line(&error_result).unwrap();
        let TurnOutcome::Failed {
            sanitized_result, ..
        } = parser.finish(true).unwrap()
        else {
            panic!("expected failure");
        };
        let sanitized = sanitized_result.unwrap();
        assert_eq!(sanitized.chars().count(), 501);
        assert!(sanitized.ends_with('…'));
    }

    #[test]
    fn success_without_init_is_a_protocol_error() {
        let mut parser = StreamParser::new(SESSION);
        parser.ingest_line(&result_line("success", "hi")).unwrap();
        let err = parser.finish(true).unwrap_err();
        assert_eq!(err.code(), ClaudeErrorCode::Protocol);
    }

    #[test]
    fn success_result_with_non_zero_exit_is_process_failure() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        parser.ingest_line(&text_delta("hi")).unwrap();
        parser.ingest_line(&result_line("success", "hi")).unwrap();
        let TurnOutcome::Failed {
            code,
            subtype,
            exit_ok,
            ..
        } = parser.finish(false).unwrap()
        else {
            panic!("expected failure");
        };
        assert_eq!(code, ClaudeErrorCode::ProcessFailed);
        assert_eq!(subtype.as_deref(), Some("success"));
        assert!(!exit_ok);
    }

    #[test]
    fn non_zero_exit_without_result_is_process_failure() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        parser.ingest_line(&text_delta("partial")).unwrap();
        let TurnOutcome::Failed {
            code,
            subtype,
            sanitized_result,
            exit_ok,
        } = parser.finish(false).unwrap()
        else {
            panic!("expected failure");
        };
        assert_eq!(code, ClaudeErrorCode::ProcessFailed);
        assert_eq!(subtype, None);
        assert_eq!(sanitized_result, None);
        assert!(!exit_ok);
    }

    #[test]
    fn unknown_types_and_fields_are_ignored() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        assert_eq!(
            parser
                .ingest_line(r#"{"type":"system","subtype":"status"}"#)
                .unwrap(),
            None
        );
        assert_eq!(
            parser
                .ingest_line(&format!(
                    r#"{{"type":"stream_event","event":{{"type":"message_start"}},"session_id":"{SESSION}"}}"#
                ))
                .unwrap(),
            None
        );
        assert_eq!(
            parser
                .ingest_line(&format!(
                    r#"{{"type":"stream_event","event":{{"type":"content_block_delta","delta":{{"type":"signature_delta","signature":"x"}}}},"session_id":"{SESSION}"}}"#
                ))
                .unwrap(),
            None
        );
        assert_eq!(
            parser
                .ingest_line(r#"{"type":"totally_new","field":1}"#)
                .unwrap(),
            None
        );
        parser.ingest_line(&result_line("success", "done")).unwrap();
        let TurnOutcome::Success { answer, .. } = parser.finish(true).unwrap() else {
            panic!("expected success");
        };
        assert_eq!(answer, "done");
    }

    #[test]
    fn non_json_line_is_a_protocol_error() {
        let mut parser = StreamParser::new(SESSION);
        let err = parser.ingest_line("this is not json").unwrap_err();
        assert_eq!(err.code(), ClaudeErrorCode::Protocol);
        assert!(err.to_string().contains("this is not"));
    }

    #[test]
    fn exit_ok_without_result_is_a_protocol_error() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        parser.ingest_line(&text_delta("partial")).unwrap();
        let err = parser.finish(true).unwrap_err();
        assert_eq!(err.code(), ClaudeErrorCode::Protocol);
    }

    #[test]
    fn result_without_session_id_is_a_protocol_error() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        let orphan = r#"{"type":"result","subtype":"success","result":"hi"}"#;
        let err = parser.ingest_line(orphan).unwrap_err();
        assert_eq!(err.code(), ClaudeErrorCode::Protocol);
    }

    #[test]
    fn result_session_mismatch_is_an_error() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        let foreign =
            result_line("success", "hi").replace(SESSION, "88888888-8888-4888-8888-888888888888");
        let err = parser.ingest_line(&foreign).unwrap_err();
        assert_eq!(err.code(), ClaudeErrorCode::SessionMismatch);
    }

    #[test]
    fn success_outcome_carries_metadata() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        parser.ingest_line(&thinking_delta("hmm")).unwrap();
        parser.ingest_line(&text_delta("hi")).unwrap();
        parser.ingest_line(&result_line("success", "hi")).unwrap();
        let TurnOutcome::Success {
            answer,
            thinking,
            cli_session_id,
            model,
            cli_version,
        } = parser.finish(true).unwrap()
        else {
            panic!("expected success");
        };
        assert_eq!(answer, "hi");
        assert_eq!(thinking, "hmm");
        assert_eq!(cli_session_id, SESSION);
        assert_eq!(model.as_deref(), Some("glm-5.3[1m]"));
        assert_eq!(cli_version.as_deref(), Some("2.1.238"));
    }

    #[test]
    fn result_text_wins_when_streamed_conflicts() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        parser.ingest_line(&text_delta("streamed")).unwrap();
        parser.ingest_line(&assistant_line("different")).unwrap();
        parser
            .ingest_line(&result_line("success", "different"))
            .unwrap();
        let TurnOutcome::Success { answer, .. } = parser.finish(true).unwrap() else {
            panic!("expected success");
        };
        assert_eq!(answer, "different");
    }

    #[test]
    fn thinking_from_assistant_blocks_is_used_without_deltas() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        let assistant = format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"thinking","thinking":"deep"}}]}},"session_id":"{SESSION}"}}"#
        );
        parser.ingest_line(&assistant).unwrap();
        parser
            .ingest_line(&result_line("success", "answer"))
            .unwrap();
        let TurnOutcome::Success { thinking, .. } = parser.finish(true).unwrap() else {
            panic!("expected success");
        };
        assert_eq!(thinking, "deep");
    }
}

#[cfg(test)]
mod process_tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    const SESSION: &str = "11111111-2222-4333-8444-555555555555";

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Fixture {
            let root = std::env::temp_dir().join(format!(
                "nest-claude-proc-test-{}-{}",
                name,
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&root).unwrap();
            Fixture { root }
        }

        fn write_fake_cli(&self, script_body: &str) -> ClaudeDetection {
            let script = self.root.join("fake-claude.cjs");
            std::fs::write(&script, script_body).unwrap();
            let node = which_node();
            ClaudeDetection {
                configured_path: script.to_string_lossy().to_string(),
                resolved_path: script.to_string_lossy().to_string(),
                launch_target: ClaudeLaunchTarget::NodeScript {
                    node_executable: node,
                    script,
                },
            }
        }

        fn vault_root(&self) -> PathBuf {
            let vault = self.root.join("vault");
            std::fs::create_dir_all(&vault).unwrap();
            vault
        }

        fn attempts(&self) -> usize {
            let path = self.vault_root().join("attempts.txt");
            std::fs::read_to_string(path)
                .map(|text| text.trim().parse().unwrap())
                .unwrap_or(0)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn which_node() -> PathBuf {
        let path_env = std::env::var("PATH").unwrap_or_default();
        let node = collect_search_dirs(&path_env)
            .into_iter()
            .map(|dir| dir.join("node.exe"))
            .find(|node| node.is_file());
        node.expect("node.exe must be available for process tests")
    }

    async fn run(
        detection: &ClaudeDetection,
        request: ClaudeTurnRequest<'_>,
    ) -> Result<ClaudeTurnResult, ClaudeTurnError> {
        let cancel = never_cancel();
        let events = TurnEvents::default();
        run_turn(detection, request, &events, &cancel).await
    }

    fn sid_helper_js() -> &'static str {
        r#"(args.includes('--session-id') ? args[args.indexOf('--session-id') + 1] : args[args.indexOf('--resume') + 1])"#
    }

    fn ok_script() -> String {
        format!(
            r#"
const args = process.argv.slice(2);
const sid = {sid_helper};
const lines = [];
lines.push(JSON.stringify({{type:'system',subtype:'init',session_id:sid,model:'fake-model',claude_code_version:'9.9.9'}}));
lines.push(JSON.stringify({{type:'stream_event',event:{{type:'content_block_delta',index:1,delta:{{type:'text_delta',text:'hel'}}}}}}));
lines.push(JSON.stringify({{type:'stream_event',event:{{type:'content_block_delta',index:1,delta:{{type:'text_delta',text:'lo'}}}}}}));
lines.push(JSON.stringify({{type:'assistant',message:{{content:[{{type:'text',text:'hello'}}]}}}}));
lines.push(JSON.stringify({{type:'result',subtype:'success',session_id:sid,result:'hello'}}));
for (const line of lines) {{ console.log(line); }}
"#,
            sid_helper = sid_helper_js()
        )
    }

    fn args_echo_script(prefix: &str) -> String {
        format!(
            r#"
const fs = require('fs');
const args = process.argv.slice(2);
const prompt = fs.readFileSync(0, 'utf8');
const sid = {sid_helper};
const mode = args.includes('--session-id') ? 'new' : (args.includes('--resume') ? 'resume' : 'none');
const flags = (args.includes('--max-turns') ? 'MAX ' : '') + (args.includes('--model') ? 'MODEL ' : '') + (args.includes('--continue') ? 'CONT ' : '');
const lines = [];
lines.push(JSON.stringify({{type:'system',subtype:'init',session_id:sid,model:'m',claude_code_version:'v'}}));
lines.push(JSON.stringify({{type:'result',subtype:'success',session_id:sid,result:'{prefix}:' + mode + ':' + prompt + ':' + flags.trim()}}));
for (const line of lines) {{ console.log(line); }}
"#,
            sid_helper = sid_helper_js(),
            prefix = prefix
        )
    }

    fn attempts_script(first_body: &str, later_body: &str) -> String {
        format!(
            r#"
const fs = require('fs');
const args = process.argv.slice(2);
const prompt = fs.readFileSync(0, 'utf8');
const sid = {sid_helper};
const attempts = fs.existsSync('attempts.txt') ? parseInt(fs.readFileSync('attempts.txt','utf8'), 10) : 0;
fs.writeFileSync('attempts.txt', String(attempts + 1));
{first_body}
{later_body}
"#,
            sid_helper = sid_helper_js(),
            first_body = first_body,
            later_body = later_body
        )
    }

    #[tokio::test]
    async fn probe_version_reads_first_token() {
        let fx = Fixture::new("probe");
        let detection = fx.write_fake_cli("console.log('3.7.1 (Claude Code)');");
        let outcome = probe_version(&detection, PROBE_VERSION_TIMEOUT).await;
        assert_eq!(outcome, ProbeOutcome::Version("3.7.1".to_string()));
    }

    #[tokio::test]
    async fn probe_version_failure_reports_exit_with_stderr_tail() {
        let fx = Fixture::new("probe-fail");
        let detection = fx.write_fake_cli("console.error('boom'); process.exit(1);");
        let outcome = probe_version(&detection, PROBE_VERSION_TIMEOUT).await;
        match outcome {
            ProbeOutcome::Failed(ProbeFailure::Exited { stderr_tail }) => {
                assert!(stderr_tail.contains("boom"))
            }
            other => panic!("expected exit failure, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn probe_version_timeout_kills_process() {
        let fx = Fixture::new("probe-timeout");
        let detection = fx.write_fake_cli("setInterval(() => {}, 1000);");
        let started = std::time::Instant::now();
        let outcome = probe_version(&detection, Duration::from_millis(300)).await;
        assert_eq!(outcome, ProbeOutcome::Failed(ProbeFailure::Timeout));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn probe_version_tolerates_huge_stderr() {
        let fx = Fixture::new("probe-noise");
        let detection = fx.write_fake_cli(
            "process.stderr.write('x'.repeat(200 * 1024)); console.log('1.0.0 (Claude Code)');",
        );
        let outcome = probe_version(&detection, PROBE_VERSION_TIMEOUT).await;
        assert_eq!(outcome, ProbeOutcome::Version("1.0.0".to_string()));
    }

    #[tokio::test]
    async fn turn_streams_tokens_and_returns_answer() {
        let fx = Fixture::new("turn-ok");
        let detection = fx.write_fake_cli(&ok_script());
        let tokens = Arc::new(std::sync::Mutex::new(Vec::new()));
        let token_sink = tokens.clone();
        let events = TurnEvents {
            token: Box::new(move |text: &str| {
                token_sink.lock().unwrap().push(text.to_string());
            }),
            ..TurnEvents::default()
        };
        let request = ClaudeTurnRequest {
            vault_root: &fx.vault_root(),
            session_id: SESSION,
            mode: TurnMode::NewSession,
            prompt: "hi",
        };
        let cancel = never_cancel();
        let result = run_turn(&detection, request, &events, &cancel)
            .await
            .unwrap();
        assert_eq!(result.answer, "hello");
        assert_eq!(result.model.as_deref(), Some("fake-model"));
        assert_eq!(result.cli_version.as_deref(), Some("9.9.9"));
        assert!(!result.used_fallback_resume);
        let streamed = tokens.lock().unwrap().join("");
        assert_eq!(streamed, "hello");
    }

    #[tokio::test]
    async fn turn_passes_args_reads_prompt_from_stdin_and_omits_limiting_flags() {
        let fx = Fixture::new("turn-args");
        let detection = fx.write_fake_cli(&args_echo_script("mode"));
        let request = ClaudeTurnRequest {
            vault_root: &fx.vault_root(),
            session_id: SESSION,
            mode: TurnMode::NewSession,
            prompt: "from stdin",
        };
        let result = run(&detection, request).await.unwrap();
        assert_eq!(result.answer, "mode:new:from stdin:");
    }

    #[tokio::test]
    async fn resume_mode_passes_resume_arg() {
        let fx = Fixture::new("turn-resume");
        let detection = fx.write_fake_cli(&args_echo_script("mode"));
        let request = ClaudeTurnRequest {
            vault_root: &fx.vault_root(),
            session_id: SESSION,
            mode: TurnMode::Resume,
            prompt: "next",
        };
        let result = run(&detection, request).await.unwrap();
        assert_eq!(result.answer, "mode:resume:next:");
    }

    #[tokio::test]
    async fn id_in_use_before_init_triggers_single_transparent_resume() {
        let fx = Fixture::new("turn-fallback");
        let script = attempts_script(
            r#"if (attempts === 0) { console.error('Error: Session ID ' + sid + ' is already in use.'); process.exit(2); }"#,
            r#"const lines = [];
lines.push(JSON.stringify({type:'system',subtype:'init',session_id:sid,model:'m',claude_code_version:'v'}));
lines.push(JSON.stringify({type:'result',subtype:'success',session_id:sid,result:'recovered:' + prompt}));
for (const line of lines) { console.log(line); }"#,
        );
        let detection = fx.write_fake_cli(&script);
        let request = ClaudeTurnRequest {
            vault_root: &fx.vault_root(),
            session_id: SESSION,
            mode: TurnMode::NewSession,
            prompt: "retry me",
        };
        let result = run(&detection, request).await.unwrap();
        assert_eq!(result.answer, "recovered:retry me");
        assert!(result.used_fallback_resume);
        assert_eq!(fx.attempts(), 2);
    }

    #[tokio::test]
    async fn id_in_use_after_init_does_not_fallback() {
        let fx = Fixture::new("turn-init-seen");
        let script = attempts_script(
            r#"console.log(JSON.stringify({type:'system',subtype:'init',session_id:sid,model:'m',claude_code_version:'v'}));
console.error('Error: Session ID ' + sid + ' is already in use.');
process.exit(2);"#,
            "",
        );
        let detection = fx.write_fake_cli(&script);
        let request = ClaudeTurnRequest {
            vault_root: &fx.vault_root(),
            session_id: SESSION,
            mode: TurnMode::NewSession,
            prompt: "hi",
        };
        let error = run(&detection, request).await.unwrap_err();
        match error {
            ClaudeTurnError::Process {
                id_in_use,
                saw_init,
                ..
            } => {
                assert!(id_in_use);
                assert!(saw_init);
            }
            other => panic!("expected Process, got {other:?}"),
        }
        assert_eq!(fx.attempts(), 1);
    }

    #[tokio::test]
    async fn fallback_no_conversation_maps_to_protocol_error() {
        let fx = Fixture::new("turn-contradiction");
        let script = attempts_script(
            r#"if (attempts === 0) { console.error('Error: Session ID ' + sid + ' is already in use.'); process.exit(2); }"#,
            r#"console.error('No conversation found with session ID: ' + sid); process.exit(1);"#,
        );
        let detection = fx.write_fake_cli(&script);
        let request = ClaudeTurnRequest {
            vault_root: &fx.vault_root(),
            session_id: SESSION,
            mode: TurnMode::NewSession,
            prompt: "hi",
        };
        let error = run(&detection, request).await.unwrap_err();
        assert!(matches!(error, ClaudeTurnError::Protocol { .. }));
        assert_eq!(fx.attempts(), 2);
    }

    #[tokio::test]
    async fn resume_mode_never_falls_back_on_id_in_use() {
        let fx = Fixture::new("turn-resume-inuse");
        let script = attempts_script(
            r#"console.error('Error: Session ID ' + sid + ' is already in use.'); process.exit(2);"#,
            "",
        );
        let detection = fx.write_fake_cli(&script);
        let request = ClaudeTurnRequest {
            vault_root: &fx.vault_root(),
            session_id: SESSION,
            mode: TurnMode::Resume,
            prompt: "hi",
        };
        let error = run(&detection, request).await.unwrap_err();
        assert!(matches!(
            error,
            ClaudeTurnError::Process {
                id_in_use: true,
                ..
            }
        ));
        assert_eq!(fx.attempts(), 1);
    }

    #[tokio::test]
    async fn error_result_becomes_cli_error_with_subtype() {
        let fx = Fixture::new("turn-error-result");
        let script = format!(
            r#"
const args = process.argv.slice(2);
const sid = {sid_helper};
const lines = [];
lines.push(JSON.stringify({{type:'system',subtype:'init',session_id:sid,model:'m',claude_code_version:'v'}}));
lines.push(JSON.stringify({{type:'result',subtype:'error_during_execution',session_id:sid,is_error:true,result:'exploded'}}));
for (const line of lines) {{ console.log(line); }}
"#,
            sid_helper = sid_helper_js()
        );
        let detection = fx.write_fake_cli(&script);
        let request = ClaudeTurnRequest {
            vault_root: &fx.vault_root(),
            session_id: SESSION,
            mode: TurnMode::NewSession,
            prompt: "hi",
        };
        let error = run(&detection, request).await.unwrap_err();
        match error {
            ClaudeTurnError::CliError {
                code,
                subtype,
                sanitized_result,
                exit_ok,
            } => {
                assert_eq!(code, ClaudeErrorCode::Protocol);
                assert_eq!(subtype, "error_during_execution");
                assert_eq!(sanitized_result.as_deref(), Some("exploded"));
                assert!(exit_ok);
            }
            other => panic!("expected CliError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn success_result_with_non_zero_exit_is_cli_error() {
        let fx = Fixture::new("turn-late-crash");
        let script = format!(
            r#"
const args = process.argv.slice(2);
const sid = {sid_helper};
const lines = [];
lines.push(JSON.stringify({{type:'system',subtype:'init',session_id:sid,model:'m',claude_code_version:'v'}}));
lines.push(JSON.stringify({{type:'result',subtype:'success',session_id:sid,result:'done'}}));
for (const line of lines) {{ console.log(line); }}
process.exit(3);
"#,
            sid_helper = sid_helper_js()
        );
        let detection = fx.write_fake_cli(&script);
        let request = ClaudeTurnRequest {
            vault_root: &fx.vault_root(),
            session_id: SESSION,
            mode: TurnMode::NewSession,
            prompt: "hi",
        };
        let error = run(&detection, request).await.unwrap_err();
        match error {
            ClaudeTurnError::CliError {
                code,
                subtype,
                exit_ok,
                ..
            } => {
                assert_eq!(code, ClaudeErrorCode::ProcessFailed);
                assert_eq!(subtype, "success");
                assert!(!exit_ok);
            }
            other => panic!("expected CliError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn non_zero_exit_without_result_is_process_error() {
        let fx = Fixture::new("turn-crash");
        let detection = fx.write_fake_cli("process.exit(1);");
        let request = ClaudeTurnRequest {
            vault_root: &fx.vault_root(),
            session_id: SESSION,
            mode: TurnMode::NewSession,
            prompt: "hi",
        };
        let error = run(&detection, request).await.unwrap_err();
        match error {
            ClaudeTurnError::Process {
                id_in_use,
                no_conversation,
                saw_init,
                ..
            } => {
                assert!(!id_in_use);
                assert!(!no_conversation);
                assert!(!saw_init);
            }
            other => panic!("expected Process, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancel_token_terminates_the_turn() {
        let fx = Fixture::new("turn-cancel");
        let detection = fx.write_fake_cli(
            "console.log(JSON.stringify({type:'system',subtype:'status'})); setInterval(() => {}, 1000);",
        );
        let cancel: CancelToken = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let request = ClaudeTurnRequest {
            vault_root: &fx.vault_root(),
            session_id: SESSION,
            mode: TurnMode::NewSession,
            prompt: "hi",
        };
        let events = TurnEvents::default();
        let cancel_for_task = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            cancel_for_task.store(true, Ordering::SeqCst);
        });
        let started = std::time::Instant::now();
        let error = run_turn(&detection, request, &events, &cancel)
            .await
            .unwrap_err();
        assert_eq!(error, ClaudeTurnError::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn spawn_failure_reports_spawn_error() {
        let fx = Fixture::new("turn-nospawn");
        let missing_node = fx.root.join("missing-node.exe");
        let detection = ClaudeDetection {
            configured_path: String::new(),
            resolved_path: String::new(),
            launch_target: ClaudeLaunchTarget::NodeScript {
                node_executable: missing_node,
                script: fx.root.join("fake.cjs"),
            },
        };
        let request = ClaudeTurnRequest {
            vault_root: &fx.vault_root(),
            session_id: SESSION,
            mode: TurnMode::NewSession,
            prompt: "hi",
        };
        let error = run(&detection, request).await.unwrap_err();
        assert!(matches!(error, ClaudeTurnError::SpawnFailed { .. }));
    }

    #[tokio::test]
    async fn initialized_callback_failure_aborts_turn() {
        let fx = Fixture::new("turn-init-persist");
        let detection = fx.write_fake_cli(&ok_script());
        let events = TurnEvents {
            initialized: Box::new(|_sid, _model, _version| Err("db write failed".to_string())),
            ..TurnEvents::default()
        };
        let request = ClaudeTurnRequest {
            vault_root: &fx.vault_root(),
            session_id: SESSION,
            mode: TurnMode::NewSession,
            prompt: "hi",
        };
        let cancel = never_cancel();
        let error = run_turn(&detection, request, &events, &cancel)
            .await
            .unwrap_err();
        assert_eq!(
            error,
            ClaudeTurnError::InitPersist {
                message: "db write failed".to_string()
            }
        );
    }

    #[tokio::test]
    async fn non_json_stdout_is_protocol_error() {
        let fx = Fixture::new("turn-garbage");
        let detection = fx.write_fake_cli("console.log('garbage line'); process.exit(0);");
        let request = ClaudeTurnRequest {
            vault_root: &fx.vault_root(),
            session_id: SESSION,
            mode: TurnMode::NewSession,
            prompt: "hi",
        };
        let error = run(&detection, request).await.unwrap_err();
        match error {
            ClaudeTurnError::Protocol { message } => assert!(message.contains("garbage")),
            other => panic!("expected Protocol, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn init_session_mismatch_aborts_turn() {
        let fx = Fixture::new("turn-mismatch");
        let script = r#"
const lines = [];
lines.push(JSON.stringify({type:'system',subtype:'init',session_id:'99999999-9999-4999-8999-999999999999',model:'m',claude_code_version:'v'}));
for (const line of lines) { console.log(line); }
"#;
        let detection = fx.write_fake_cli(script);
        let request = ClaudeTurnRequest {
            vault_root: &fx.vault_root(),
            session_id: SESSION,
            mode: TurnMode::NewSession,
            prompt: "hi",
        };
        let error = run(&detection, request).await.unwrap_err();
        assert!(matches!(error, ClaudeTurnError::SessionMismatch { .. }));
    }

    #[test]
    fn error_matchers_ignore_ansi_and_match_real_cli_text() {
        assert!(is_session_id_in_use_error(
            "Error: Session ID 6f1c2a9e-... is already in use."
        ));
        assert!(is_session_id_in_use_error(
            "\u{1b}[31mError: Session ID x is already in use.\u{1b}[0m"
        ));
        assert!(!is_session_id_in_use_error("some other failure"));
        assert!(is_no_conversation_found_error(
            "No conversation found with session ID: 00000000-..."
        ));
        assert!(!is_no_conversation_found_error(
            "Session ID x is already in use."
        ));
    }

    #[test]
    fn strip_ansi_removes_escape_sequences() {
        assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m"), "red");
        assert_eq!(strip_ansi("plain"), "plain");
    }
}
