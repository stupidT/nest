#![allow(dead_code)]

use crate::error::{AppError, AppResult};
use std::path::{Path, PathBuf};

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

    pub fn into_error(self, detail: impl std::fmt::Display) -> AppError {
        AppError::msg(format!("{}: {}", self.as_str(), detail))
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
pub struct ClaudeDetection {
    pub configured_path: String,
    pub resolved_path: String,
    pub launch_target: ClaudeLaunchTarget,
}

pub fn detect_cli(configured_path: Option<&Path>) -> AppResult<Vec<ClaudeDetection>> {
    match configured_path {
        Some(path) if !path.as_os_str().is_empty() => {
            let home = std::env::var("USERPROFILE")
                .map(PathBuf::from)
                .unwrap_or_default();
            let normalized = normalize_configured(&std::env::current_dir()?, &home, path);
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

fn detect_auto() -> AppResult<Vec<ClaudeDetection>> {
    let path_env = std::env::var("PATH").unwrap_or_default();
    let search_dirs = collect_search_dirs(&path_env);
    let candidates = find_auto_candidates(&search_dirs, &default_npm_global_dirs(), &path_env);
    detections_from_candidates(candidates)
}

fn detections_from_candidates(
    candidates: Vec<(PathBuf, ClaudeLaunchTarget)>,
) -> AppResult<Vec<ClaudeDetection>> {
    if candidates.is_empty() {
        return Err(ClaudeErrorCode::InvalidCliPath
            .into_error("no Claude CLI found on PATH or in known npm locations"));
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
) -> AppResult<Vec<(PathBuf, ClaudeLaunchTarget)>> {
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

fn build_node_target(wrapper: &Path, path_env: &str) -> AppResult<ClaudeLaunchTarget> {
    let node = find_node_executable(&node_hints(wrapper), path_env).ok_or_else(|| {
        ClaudeErrorCode::NodeNotFound.into_error(format!(
            "node.exe is required to launch {}",
            wrapper.display()
        ))
    })?;
    Ok(ClaudeLaunchTarget::NodeScript {
        node_executable: node,
        script: wrapper.to_path_buf(),
    })
}

fn invalid_path(path: &Path) -> AppError {
    ClaudeErrorCode::InvalidCliPath.into_error(path.display().to_string())
}

fn wrapper_relative() -> PathBuf {
    PathBuf::from("node_modules")
        .join("@anthropic-ai")
        .join("claude-code")
        .join("cli-wrapper.cjs")
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
        message: String,
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
    cli_session_id: Option<String>,
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
    let mut shown: String = text.chars().take(80).collect();
    if shown.len() < text.len() {
        shown.push('…');
    }
    shown
}

fn pick_final(streamed: &str, candidates: &[Option<&String>]) -> String {
    if streamed.is_empty() {
        for candidate in candidates.iter().flatten() {
            if !candidate.is_empty() {
                return (*candidate).clone();
            }
        }
        return String::new();
    }
    let mut best: Option<&String> = None;
    for candidate in candidates.iter().flatten() {
        let compatible = candidate.starts_with(streamed) || candidate.as_str() == streamed;
        if compatible && best.is_none_or(|current| candidate.len() > current.len()) {
            best = Some(candidate);
        }
    }
    best.map(String::from)
        .unwrap_or_else(|| streamed.to_string())
}

impl StreamParser {
    pub fn new(expected_session_id: &str) -> Self {
        Self {
            expected_session_id: expected_session_id.to_string(),
            state: ParserState::default(),
        }
    }

    pub fn ingest_line(&mut self, line: &str) -> AppResult<Option<ParserEvent>> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        let value: serde_json::Value = serde_json::from_str(trimmed).map_err(|_| {
            ClaudeErrorCode::Protocol
                .into_error(format!("non-JSON output line: {}", preview(trimmed)))
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

    fn handle_system(&mut self, value: &serde_json::Value) -> AppResult<Option<ParserEvent>> {
        if value.get("subtype").and_then(|value| value.as_str()) != Some("init") {
            return Ok(None);
        }
        let session_id = value
            .get("session_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        if session_id != self.expected_session_id {
            return Err(ClaudeErrorCode::SessionMismatch.into_error(format!(
                "init session {session_id} does not match {}",
                self.expected_session_id
            )));
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
        self.state.cli_session_id = Some(session_id.clone());
        self.state.model = model.clone();
        self.state.cli_version = cli_version.clone();
        Ok(Some(ParserEvent::Initialized {
            session_id,
            model,
            cli_version,
        }))
    }

    fn handle_stream_event(&mut self, value: &serde_json::Value) -> AppResult<Option<ParserEvent>> {
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

    fn handle_result(&mut self, value: &serde_json::Value) -> AppResult<()> {
        let session_id = value
            .get("session_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        if !session_id.is_empty() && session_id != self.expected_session_id {
            return Err(ClaudeErrorCode::SessionMismatch.into_error(format!(
                "result session {session_id} does not match {}",
                self.expected_session_id
            )));
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

    pub fn finish(&mut self, exit_ok: bool) -> AppResult<TurnOutcome> {
        let Some(result) = self.state.result.clone() else {
            return Err(ClaudeErrorCode::Protocol.into_error(format!(
                "CLI exited without a result message (exit_ok={exit_ok})"
            )));
        };
        if !exit_ok {
            return Ok(TurnOutcome::Failed {
                code: ClaudeErrorCode::ProcessFailed,
                message: format!(
                    "CLI exited unsuccessfully after result subtype {}",
                    result.subtype
                ),
            });
        }
        if !self.state.saw_valid_init {
            return Err(
                ClaudeErrorCode::Protocol.into_error("result without a matching system/init")
            );
        }
        if result.is_error || result.subtype != "success" {
            return Ok(TurnOutcome::Failed {
                code: ClaudeErrorCode::Protocol,
                message: format!("result subtype: {}", result.subtype),
            });
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
        assert_eq!(err_code(&err), ClaudeErrorCode::InvalidCliPath);
    }

    #[test]
    fn missing_file_is_invalid() {
        let fx = Fixture::new("missing");
        let err = detect_cli(Some(&fx.file("nope/claude.exe"))).unwrap_err();
        assert_eq!(err_code(&err), ClaudeErrorCode::InvalidCliPath);
    }

    #[test]
    fn unsupported_file_is_invalid() {
        let fx = Fixture::new("unsupported");
        let txt = fx.touch("claude.txt");
        let err = detect_cli(Some(&txt)).unwrap_err();
        assert_eq!(err_code(&err), ClaudeErrorCode::InvalidCliPath);
    }

    #[test]
    fn arbitrary_exe_files_are_rejected() {
        let fx = Fixture::new("evil-exe");
        let exe = fx.touch("tools/evil.exe");
        let err = detect_cli(Some(&exe)).unwrap_err();
        assert_eq!(err_code(&err), ClaudeErrorCode::InvalidCliPath);
    }

    #[test]
    fn arbitrary_scripts_are_rejected() {
        let fx = Fixture::new("evil-cjs");
        let script = fx.touch("tools/other.cjs");
        let err = detect_cli(Some(&script)).unwrap_err();
        assert_eq!(err_code(&err), ClaudeErrorCode::InvalidCliPath);
    }

    #[test]
    fn non_claude_shims_are_rejected() {
        let fx = Fixture::new("evil-cmd");
        let shim = fx.touch("tools/not-claude.cmd");
        let err = detect_cli(Some(&shim)).unwrap_err();
        assert_eq!(err_code(&err), ClaudeErrorCode::InvalidCliPath);
    }

    #[test]
    fn node_missing_fails_the_wrapper_resolution() {
        let fx = Fixture::new("no-node");
        let wrapper = fx.touch("npm/node_modules/@anthropic-ai/claude-code/cli-wrapper.cjs");
        let err = resolve_entry(&wrapper, "").unwrap_err();
        assert_eq!(err_code(&err), ClaudeErrorCode::NodeNotFound);
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
        assert_eq!(err_code(&err), ClaudeErrorCode::InvalidCliPath);
    }

    fn err_code(error: &AppError) -> ClaudeErrorCode {
        let text = error.to_string();
        [
            ClaudeErrorCode::InvalidCliPath,
            ClaudeErrorCode::NodeNotFound,
            ClaudeErrorCode::Protocol,
            ClaudeErrorCode::SessionMismatch,
            ClaudeErrorCode::ProcessFailed,
        ]
        .into_iter()
        .find(|code| text.starts_with(code.as_str()))
        .expect("error should carry a ClaudeErrorCode prefix")
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
        let err = parser.finish(true).unwrap_err();
        assert!(err.to_string().contains(ClaudeErrorCode::Protocol.as_str()));
    }

    #[test]
    fn init_session_mismatch_is_an_error() {
        let mut parser = StreamParser::new(SESSION);
        let foreign = r#"{"type":"system","subtype":"init","session_id":"88888888-8888-4888-8888-888888888888","model":"m"}"#;
        let err = parser.ingest_line(foreign).unwrap_err();
        assert!(err
            .to_string()
            .contains(ClaudeErrorCode::SessionMismatch.as_str()));
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
    fn result_error_subtype_fails_the_turn() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        let error_result = format!(
            r#"{{"type":"result","subtype":"error_during_execution","session_id":"{SESSION}","is_error":true,"result":"boom"}}"#
        );
        parser.ingest_line(&error_result).unwrap();
        let TurnOutcome::Failed { code, message } = parser.finish(true).unwrap() else {
            panic!("expected failure");
        };
        assert_eq!(code, ClaudeErrorCode::Protocol);
        assert!(message.contains("error_during_execution"));
    }

    #[test]
    fn success_without_init_is_a_protocol_error() {
        let mut parser = StreamParser::new(SESSION);
        parser.ingest_line(&result_line("success", "hi")).unwrap();
        let err = parser.finish(true).unwrap_err();
        assert!(err.to_string().contains(ClaudeErrorCode::Protocol.as_str()));
    }

    #[test]
    fn success_result_with_non_zero_exit_is_process_failure() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        parser.ingest_line(&text_delta("hi")).unwrap();
        parser.ingest_line(&result_line("success", "hi")).unwrap();
        let TurnOutcome::Failed { code, .. } = parser.finish(false).unwrap() else {
            panic!("expected failure");
        };
        assert_eq!(code, ClaudeErrorCode::ProcessFailed);
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
        assert!(err.to_string().contains(ClaudeErrorCode::Protocol.as_str()));
        assert!(err.to_string().contains("this is not"));
    }

    #[test]
    fn exit_ok_without_result_is_a_protocol_error() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        parser.ingest_line(&text_delta("partial")).unwrap();
        let err = parser.finish(true).unwrap_err();
        assert!(err.to_string().contains(ClaudeErrorCode::Protocol.as_str()));
    }

    #[test]
    fn non_zero_exit_without_result_reports_missing_result() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        let err = parser.finish(false).unwrap_err();
        assert!(err.to_string().contains(ClaudeErrorCode::Protocol.as_str()));
    }

    #[test]
    fn result_session_mismatch_is_an_error() {
        let mut parser = StreamParser::new(SESSION);
        feed_init(&mut parser);
        let foreign =
            result_line("success", "hi").replace(SESSION, "88888888-8888-4888-8888-888888888888");
        let err = parser.ingest_line(&foreign).unwrap_err();
        assert!(err
            .to_string()
            .contains(ClaudeErrorCode::SessionMismatch.as_str()));
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
    fn streamed_text_wins_when_candidates_conflict() {
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
        assert_eq!(answer, "streamed");
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
