//! Claude CLI integration for the Claude Agent chat backend.
//!
//! This module owns Windows executable resolution, subprocess orchestration,
//! NDJSON stream parsing, and process-tree termination. Step 1 scope covers
//! resolution; probe/run/kill land in later slices.
#![allow(dead_code)] // wired up by commands/claude.rs in a later Step 1 slice

use crate::error::{AppError, AppResult};
use std::path::{Path, PathBuf};

pub const ERR_INVALID_CLI_PATH: &str = "invalid_cli_path";
pub const ERR_NODE_NOT_FOUND: &str = "node_not_found";
pub const ERR_CLAUDE_PROTOCOL: &str = "claude_protocol_error";
pub const ERR_CLAUDE_SESSION_MISMATCH: &str = "claude_session_mismatch";

/// How the resolved Claude CLI must be spawned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaudeLaunchTarget {
    /// A native executable (`claude.exe`) spawned directly.
    Executable { executable: PathBuf },
    /// An npm install spawned as `node.exe <.../claude-code/cli-wrapper.cjs>`.
    NodeScript {
        node_executable: PathBuf,
        script: PathBuf,
    },
}

/// The result of resolving a Claude CLI configuration into a launchable form.
#[derive(Debug, Clone)]
pub struct ClaudeDetection {
    /// Normalized user input (empty when auto-detecting).
    pub configured_path: String,
    /// The path actually launched: the executable or the wrapper script.
    pub resolved_path: String,
    pub launch_target: ClaudeLaunchTarget,
}

/// Resolves the configured CLI path, or auto-detects when `configured_path`
/// is `None`/empty. Follows the manual path rules in the Step 1 design:
/// `claude.exe` launches directly, `cli-wrapper.cjs` launches via Node, and
/// `.cmd`/`.ps1`/extensionless npm shims are resolved to their underlying
/// wrapper rather than executed through a shell.
pub fn detect_cli(configured_path: Option<&Path>) -> AppResult<ClaudeDetection> {
    match configured_path {
        Some(path) if !path.as_os_str().is_empty() => {
            let home = std::env::var("USERPROFILE")
                .map(PathBuf::from)
                .unwrap_or_default();
            let normalized = normalize_configured(&std::env::current_dir()?, &home, path);
            let path_env = std::env::var("PATH").unwrap_or_default();
            let (resolved, target) = resolve_entry(&normalized, &path_env)?;
            Ok(ClaudeDetection {
                configured_path: normalized.to_string_lossy().to_string(),
                resolved_path: resolved.to_string_lossy().to_string(),
                launch_target: target,
            })
        }
        _ => detect_auto(),
    }
}

fn detect_auto() -> AppResult<ClaudeDetection> {
    let path_env = std::env::var("PATH").unwrap_or_default();
    let search_dirs = collect_search_dirs(&path_env);
    let (resolved, target) =
        find_auto_candidate(&search_dirs, &default_npm_global_dirs(), &path_env)?;
    Ok(ClaudeDetection {
        configured_path: String::new(),
        resolved_path: resolved.to_string_lossy().to_string(),
        launch_target: target,
    })
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

/// Normalizes a configured path against `base` (the working directory in
/// production). Relative inputs become absolute; `~` expands to `home`.
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

/// Resolves a concrete filesystem entry (file or directory) into a launch
/// target. Directories probe `claude.exe` first, then the npm wrapper layout.
/// `path_env` is injected so tests are independent of the host PATH.
pub(crate) fn resolve_entry(
    path: &Path,
    path_env: &str,
) -> AppResult<(PathBuf, ClaudeLaunchTarget)> {
    if path.is_dir() {
        let exe = path.join("claude.exe");
        if exe.is_file() {
            return Ok((
                exe.clone(),
                ClaudeLaunchTarget::Executable { executable: exe },
            ));
        }
        let wrapper = path.join(wrapper_relative());
        if wrapper.is_file() {
            let target = build_node_target(&wrapper, path_env)?;
            return Ok((wrapper, target));
        }
        return Err(invalid_path(path));
    }
    if !path.is_file() {
        return Err(invalid_path(path));
    }
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match extension.as_deref() {
        Some("exe") => Ok((
            path.to_path_buf(),
            ClaudeLaunchTarget::Executable {
                executable: path.to_path_buf(),
            },
        )),
        Some("cjs") | Some("js") => {
            let target = build_node_target(path, path_env)?;
            Ok((path.to_path_buf(), target))
        }
        // npm shims (`.cmd`, `.bat`, `.ps1`) and extensionless POSIX shims
        // are never executed through a shell; they only point at the wrapper.
        Some("cmd") | Some("bat") | Some("ps1") | None => {
            let wrapper = resolve_shim(path).ok_or_else(|| invalid_path(path))?;
            let target = build_node_target(&wrapper, path_env)?;
            Ok((wrapper, target))
        }
        Some(_) => Err(invalid_path(path)),
    }
}

/// Resolves an npm shim (`.cmd`, `.ps1`, or extensionless) to its underlying
/// `cli-wrapper.cjs`. Structure inference first (shim dir + `node_modules`),
/// then shim-content parsing as a fallback. Returns the wrapper script.
pub(crate) fn resolve_shim(shim: &Path) -> Option<PathBuf> {
    let dir = shim.parent()?;
    let structural = dir.join(wrapper_relative());
    if structural.is_file() {
        return Some(structural);
    }
    let content = std::fs::read_to_string(shim).ok()?;
    parse_shim_content_for_wrapper(dir, &content)
}

/// Extracts a wrapper script path referenced by shim file content.
/// `%~dp0` and `$basedir` anchors resolve relative to `shim_dir`.
pub(crate) fn parse_shim_content_for_wrapper(shim_dir: &Path, content: &str) -> Option<PathBuf> {
    for line in content.lines() {
        let line = line.trim();
        if !line.contains("cli-wrapper.cjs") && !line.contains("cli.js") {
            continue;
        }
        for token in line.split_whitespace() {
            if !(token.contains("cli-wrapper.cjs") || token.contains("cli.js")) {
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

/// Finds a `node.exe`: hinted directories first, then `path_env` entries.
/// Exists-check only; execution is probed in a later slice.
pub(crate) fn find_node_executable(hints: &[PathBuf], path_env: &str) -> Option<PathBuf> {
    for dir in hints.iter().chain(collect_search_dirs(path_env).iter()) {
        let node = dir.join("node.exe");
        if node.is_file() {
            return Some(node);
        }
    }
    None
}

/// Candidate directories for `node.exe` near a wrapper: the wrapper's own
/// directory plus the npm install root that contains `node_modules`.
fn node_hints(wrapper: &Path) -> Vec<PathBuf> {
    let mut hints = Vec::new();
    if let Some(parent) = wrapper.parent() {
        hints.push(parent.to_path_buf());
    }
    let mut current = wrapper.parent().map(|p| p.to_path_buf());
    while let Some(dir) = current {
        if dir.file_name().and_then(|n| n.to_str()) == Some("node_modules") {
            if let Some(root) = dir.parent() {
                hints.push(root.to_path_buf());
            }
            break;
        }
        current = dir.parent().map(|p| p.to_path_buf());
    }
    hints
}

/// Splits a PATH-style environment string into candidate directories.
pub(crate) fn collect_search_dirs(path_env: &str) -> Vec<PathBuf> {
    let separator = if cfg!(windows) { ';' } else { ':' };
    path_env
        .split(separator)
        .filter(|entry| !entry.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Walks auto-detect candidates in order: PATH `claude.exe`, PATH npm shims,
/// then well-known npm global wrapper locations. Returns the first entry that
/// resolves into a launch target.
pub(crate) fn find_auto_candidate(
    search_dirs: &[PathBuf],
    npm_global_dirs: &[PathBuf],
    path_env: &str,
) -> AppResult<(PathBuf, ClaudeLaunchTarget)> {
    for dir in search_dirs {
        let exe = dir.join("claude.exe");
        if exe.is_file() {
            return Ok((
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
                        return Ok((wrapper, target));
                    }
                }
            }
        }
    }
    for dir in npm_global_dirs {
        let wrapper = dir.join(wrapper_relative());
        if wrapper.is_file() {
            if let Ok(target) = build_node_target(&wrapper, path_env) {
                return Ok((wrapper, target));
            }
        }
    }
    Err(AppError::msg(format!(
        "{ERR_INVALID_CLI_PATH}: no Claude CLI found on PATH or in known npm locations"
    )))
}

fn build_node_target(wrapper: &Path, path_env: &str) -> AppResult<ClaudeLaunchTarget> {
    let node = find_node_executable(&node_hints(wrapper), path_env).ok_or_else(|| {
        AppError::msg(format!(
            "{ERR_NODE_NOT_FOUND}: node.exe is required to launch {}",
            wrapper.display()
        ))
    })?;
    Ok(ClaudeLaunchTarget::NodeScript {
        node_executable: node,
        script: wrapper.to_path_buf(),
    })
}

fn invalid_path(path: &Path) -> AppError {
    AppError::msg(format!("{ERR_INVALID_CLI_PATH}: {}", path.display()))
}

fn wrapper_relative() -> PathBuf {
    PathBuf::from("node_modules")
        .join("@anthropic-ai")
        .join("claude-code")
        .join("cli-wrapper.cjs")
}

/// A UI-visible streaming event emitted while parsing Claude NDJSON output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParserEvent {
    Token(String),
    Thinking(String),
}

/// The terminal state of one Claude turn, derived from the `result` message.
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
        code: &'static str,
        message: String,
    },
}

/// Incremental parser for Claude CLI `--output-format stream-json` lines.
///
/// Enforces the session-ID contract, deduplicates partial deltas against the
/// final assistant/result text, and tolerates unknown event types. The
/// `result` message is the only success terminal state.
pub struct StreamParser {
    expected_session_id: String,
    state: ParserState,
}

#[derive(Default)]
struct ParserState {
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

/// Chooses the final text per the Step 1 dedup rules: with deltas streamed,
/// prefer the most complete candidate that has the streamed text as a prefix;
/// without deltas, prefer the result text, then the assistant candidate.
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

    /// Ingests one stdout line. Returns UI events to forward, or an error
    /// for protocol violations (non-JSON line, session mismatch).
    pub fn ingest_line(&mut self, line: &str) -> AppResult<Option<ParserEvent>> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        let value: serde_json::Value = serde_json::from_str(trimmed).map_err(|_| {
            AppError::msg(format!(
                "{ERR_CLAUDE_PROTOCOL}: non-JSON output line: {}",
                preview(trimmed)
            ))
        })?;
        let kind = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
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
        if value.get("subtype").and_then(|v| v.as_str()) != Some("init") {
            return Ok(None);
        }
        let session_id = value
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if session_id != self.expected_session_id {
            return Err(AppError::msg(format!(
                "{ERR_CLAUDE_SESSION_MISMATCH}: init session {session_id} does not match {}",
                self.expected_session_id
            )));
        }
        self.state.cli_session_id = Some(session_id);
        self.state.model = value
            .get("model")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        self.state.cli_version = value
            .get("claude_code_version")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        Ok(None)
    }

    fn handle_stream_event(&mut self, value: &serde_json::Value) -> AppResult<Option<ParserEvent>> {
        let event = match value.get("event") {
            Some(event) => event,
            None => return Ok(None),
        };
        if event.get("type").and_then(|v| v.as_str()) != Some("content_block_delta") {
            return Ok(None);
        }
        let delta = match event.get("delta") {
            Some(delta) => delta,
            None => return Ok(None),
        };
        match delta.get("type").and_then(|v| v.as_str()) {
            Some("text_delta") => {
                let text = delta
                    .get("text")
                    .and_then(|v| v.as_str())
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
                    .and_then(|v| v.as_str())
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
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
        else {
            return;
        };
        let mut text = String::new();
        let mut thinking = String::new();
        for block in content {
            match block.get("type").and_then(|v| v.as_str()) {
                Some("text") => {
                    if let Some(chunk) = block.get("text").and_then(|v| v.as_str()) {
                        text.push_str(chunk);
                    }
                }
                Some("thinking") => {
                    if let Some(chunk) = block.get("thinking").and_then(|v| v.as_str()) {
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
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if !session_id.is_empty() && session_id != self.expected_session_id {
            return Err(AppError::msg(format!(
                "{ERR_CLAUDE_SESSION_MISMATCH}: result session {session_id} does not match {}",
                self.expected_session_id
            )));
        }
        self.state.result = Some(ParsedResultMessage {
            subtype: value
                .get("subtype")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            session_id,
            is_error: value
                .get("is_error")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            text: value
                .get("result")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        });
        Ok(())
    }

    /// Finalizes the turn after the process exited with `exit_ok`.
    /// `exit_ok = true` without a `result` message is a protocol error.
    pub fn finish(&mut self, exit_ok: bool) -> AppResult<TurnOutcome> {
        let Some(result) = self.state.result.clone() else {
            return Err(AppError::msg(format!(
                "{ERR_CLAUDE_PROTOCOL}: CLI exited without a result message (exit_ok={exit_ok})"
            )));
        };
        if result.is_error || result.subtype != "success" {
            return Ok(TurnOutcome::Failed {
                code: ERR_CLAUDE_PROTOCOL,
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

        /// A minimal npm layout: `shim.cmd`/`shim.ps1`/`shim` beside a
        /// `node_modules/@anthropic-ai/claude-code/cli-wrapper.cjs` plus a
        /// local `node.exe` next to the shim.
        fn npm_layout(&self, dir: &str) -> PathBuf {
            let base = self.root.join(dir);
            let shim_dir = base.join("bin");
            self.touch(
                &format!("{dir}/bin/node_modules/@anthropic-ai/claude-code/cli-wrapper.cjs")
                    .replace('\\', "/"),
            );
            self.touch(&format!("{dir}/bin/node.exe").replace('\\', "/"));
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
        let detection = detect_cli(Some(&exe)).unwrap();
        assert_eq!(
            detection.launch_target,
            ClaudeLaunchTarget::Executable {
                executable: exe.clone()
            }
        );
        assert_eq!(detection.resolved_path, exe.to_string_lossy().to_string());
    }

    #[test]
    fn explicit_wrapper_script_launches_via_node() {
        let fx = Fixture::new("explicit-wrapper");
        let wrapper = fx.touch("npm/node_modules/@anthropic-ai/claude-code/cli-wrapper.cjs");
        let node = fx.touch("npm/node.exe");
        // Wrapper sits next to node.exe (hinted directory).
        let detection = detect_cli(Some(&wrapper)).unwrap();
        match detection.launch_target {
            ClaudeLaunchTarget::NodeScript {
                node_executable,
                script,
            } => {
                assert_eq!(script, wrapper);
                assert_eq!(node_executable, node);
            }
            other => panic!("expected NodeScript, got {other:?}"),
        }
    }

    #[test]
    fn cmd_shim_resolves_to_wrapper() {
        let fx = Fixture::new("cmd-shim");
        let shim_dir = fx.npm_layout("install");
        let shim = shim_dir.join("claude.cmd");
        std::fs::write(&shim, b"@echo off\r\nnode \"%~dp0\\node_modules\\@anthropic-ai\\claude-code\\cli-wrapper.cjs\" %*\r\n").unwrap();
        let detection = detect_cli(Some(&shim)).unwrap();
        match detection.launch_target {
            ClaudeLaunchTarget::NodeScript { script, .. } => {
                assert!(script.ends_with("cli-wrapper.cjs"));
            }
            other => panic!("expected NodeScript, got {other:?}"),
        }
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
        let detection = detect_cli(Some(&shim)).unwrap();
        assert!(matches!(
            detection.launch_target,
            ClaudeLaunchTarget::NodeScript { .. }
        ));
    }

    #[test]
    fn extensionless_shim_resolves_to_wrapper() {
        let fx = Fixture::new("extless-shim");
        let shim_dir = fx.npm_layout("install");
        let shim = shim_dir.join("claude");
        std::fs::write(&shim, b"#!/bin/sh\n").unwrap();
        let detection = detect_cli(Some(&shim)).unwrap();
        assert!(matches!(
            detection.launch_target,
            ClaudeLaunchTarget::NodeScript { .. }
        ));
    }

    #[test]
    fn shim_content_fallback_when_layout_is_unusual() {
        let fx = Fixture::new("shim-content");
        // Wrapper NOT in the standard sibling node_modules layout.
        let wrapper = fx.touch("elsewhere/@anthropic-ai/claude-code/cli-wrapper.cjs");
        let shim = fx.file("bin/claude.cmd");
        std::fs::create_dir_all(shim.parent().unwrap()).unwrap();
        let content =
            "@node \"%~dp0\\..\\elsewhere\\@anthropic-ai\\claude-code\\cli-wrapper.cjs\" %*\r\n";
        std::fs::write(&shim, content).unwrap();
        let parsed =
            parse_shim_content_for_wrapper(shim.parent().unwrap(), &shim_content(&shim)).unwrap();
        assert_eq!(parsed, wrapper);
    }

    fn shim_content(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn directory_prefers_native_exe() {
        let fx = Fixture::new("dir-exe");
        let dir = fx.root.join("dir");
        let exe = fx.touch("dir/claude.exe");
        fx.touch("dir/node_modules/@anthropic-ai/claude-code/cli-wrapper.cjs");
        let detection = detect_cli(Some(&dir)).unwrap();
        assert_eq!(
            detection.launch_target,
            ClaudeLaunchTarget::Executable { executable: exe }
        );
    }

    #[test]
    fn directory_falls_back_to_wrapper_layout() {
        let fx = Fixture::new("dir-wrapper");
        let dir = fx.root.join("dir");
        fx.touch("dir/node_modules/@anthropic-ai/claude-code/cli-wrapper.cjs");
        fx.touch("dir/node.exe");
        let detection = detect_cli(Some(&dir)).unwrap();
        assert!(matches!(
            detection.launch_target,
            ClaudeLaunchTarget::NodeScript { .. }
        ));
    }

    #[test]
    fn directory_without_candidates_is_invalid() {
        let fx = Fixture::new("dir-empty");
        let dir = fx.root.join("dir");
        std::fs::create_dir_all(&dir).unwrap();
        let err = detect_cli(Some(&dir)).unwrap_err();
        assert!(err.to_string().contains(ERR_INVALID_CLI_PATH));
    }

    #[test]
    fn missing_file_is_invalid() {
        let fx = Fixture::new("missing");
        let err = detect_cli(Some(&fx.file("nope/claude.exe"))).unwrap_err();
        assert!(err.to_string().contains(ERR_INVALID_CLI_PATH));
    }

    #[test]
    fn unsupported_file_is_invalid() {
        let fx = Fixture::new("unsupported");
        let txt = fx.touch("claude.txt");
        let err = detect_cli(Some(&txt)).unwrap_err();
        assert!(err.to_string().contains(ERR_INVALID_CLI_PATH));
    }

    #[test]
    fn node_missing_fails_the_wrapper_resolution() {
        let fx = Fixture::new("no-node");
        // No node.exe next to the wrapper and an empty PATH.
        let wrapper = fx.touch("npm/node_modules/@anthropic-ai/claude-code/cli-wrapper.cjs");
        let err = resolve_entry(&wrapper, "").unwrap_err();
        assert!(err.to_string().contains(ERR_NODE_NOT_FOUND));
        // The same wrapper with a node.exe hint resolves as a NodeScript.
        fx.touch("npm/node.exe");
        let (_, target) = resolve_entry(&wrapper, "").unwrap();
        assert!(matches!(target, ClaudeLaunchTarget::NodeScript { .. }));
    }

    #[test]
    fn paths_with_spaces_resolve() {
        let fx = Fixture::new("spaces");
        let exe = fx.touch("my tools/claude claude.exe");
        let detection = detect_cli(Some(&exe)).unwrap();
        assert_eq!(
            detection.launch_target,
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
    fn auto_detection_prefers_path_exe_over_shim() {
        let fx = Fixture::new("auto-order");
        let dir_a = fx.root.join("a"); // contains shim layout
        fx.touch("a/claude.cmd");
        fx.touch("a/node_modules/@anthropic-ai/claude-code/cli-wrapper.cjs");
        fx.touch("a/node.exe");
        let dir_b = fx.root.join("b"); // contains native exe
        let exe = fx.touch("b/claude.exe");

        // dir order: a (shim) then b (exe) — but exe wins by rule order per
        // directory? No: rule 1 scans ALL dirs for claude.exe first.
        let (resolved, target) = find_auto_candidate(
            &[dir_a, dir_b],
            &[],
            &format!("{}", fx.root.join("a").to_string_lossy()),
        )
        .unwrap();
        assert_eq!(resolved, exe);
        assert!(matches!(target, ClaudeLaunchTarget::Executable { .. }));
    }

    #[test]
    fn auto_detection_uses_path_shim_when_no_exe() {
        let fx = Fixture::new("auto-shim");
        let dir_a = fx.root.join("a");
        fx.touch("a/claude.cmd");
        let wrapper = fx.touch("a/node_modules/@anthropic-ai/claude-code/cli-wrapper.cjs");
        fx.touch("a/node.exe");
        let (resolved, target) =
            find_auto_candidate(std::slice::from_ref(&dir_a), &[], &dir_a.to_string_lossy())
                .unwrap();
        assert_eq!(resolved, wrapper);
        assert!(matches!(target, ClaudeLaunchTarget::NodeScript { .. }));
    }

    #[test]
    fn auto_detection_falls_back_to_npm_global_wrapper() {
        let fx = Fixture::new("auto-global");
        let global = fx.root.join("npm-global");
        let wrapper = fx.touch("npm-global/node_modules/@anthropic-ai/claude-code/cli-wrapper.cjs");
        let node = fx.touch("npm-global/node.exe");
        let (resolved, target) = find_auto_candidate(&[], &[global], "").unwrap();
        assert_eq!(resolved, wrapper);
        assert!(matches!(
            target,
            ClaudeLaunchTarget::NodeScript { script, node_executable } if script == wrapper && node_executable == node
        ));
    }

    #[test]
    fn auto_detection_failure_is_invalid_path() {
        let err = find_auto_candidate(&[], &[], "").unwrap_err();
        assert!(err.to_string().contains(ERR_INVALID_CLI_PATH));
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
    fn init_line_is_validated_and_metadata_captured() {
        let mut parser = StreamParser::new(SESSION);
        assert_eq!(parser.ingest_line(&init_line()).unwrap(), None);
        let outcome = parser.finish(true).unwrap_err();
        // No result yet: init alone is not a terminal state.
        assert!(outcome.to_string().contains(ERR_CLAUDE_PROTOCOL));
    }

    #[test]
    fn init_session_mismatch_is_an_error() {
        let mut parser = StreamParser::new(SESSION);
        let foreign = r#"{"type":"system","subtype":"init","session_id":"88888888-8888-4888-8888-888888888888","model":"m"}"#;
        let err = parser.ingest_line(foreign).unwrap_err();
        assert!(err.to_string().contains(ERR_CLAUDE_SESSION_MISMATCH));
    }

    #[test]
    fn text_deltas_emit_tokens() {
        let mut parser = StreamParser::new(SESSION);
        parser.ingest_line(&init_line()).unwrap();
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
        parser.ingest_line(&init_line()).unwrap();
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
        parser.ingest_line(&init_line()).unwrap();
        parser.ingest_line(&text_delta("hello")).unwrap();
        parser.ingest_line(&text_delta(" world")).unwrap();
        // Assistant full message and result both carry the complete text.
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
        parser.ingest_line(&init_line()).unwrap();
        // No partial deltas; candidates are cached silently.
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
        parser.ingest_line(&init_line()).unwrap();
        let error_result = r#"{"type":"result","subtype":"error_during_execution","session_id":"SESSION","is_error":true,"result":"boom"}"#.replace("SESSION", SESSION);
        parser.ingest_line(&error_result).unwrap();
        let TurnOutcome::Failed { code, message } = parser.finish(true).unwrap() else {
            panic!("expected failure");
        };
        assert_eq!(code, ERR_CLAUDE_PROTOCOL);
        assert!(message.contains("error_during_execution"));
    }

    #[test]
    fn unknown_types_and_fields_are_ignored() {
        let mut parser = StreamParser::new(SESSION);
        parser.ingest_line(&init_line()).unwrap();
        assert_eq!(
            parser
                .ingest_line(r#"{"type":"system","subtype":"status"}"#)
                .unwrap(),
            None
        );
        assert_eq!(
            parser.ingest_line(
                r#"{"type":"stream_event","event":{"type":"message_start"},"session_id":"11111111-2222-4333-8444-555555555555"}"#
            )
            .unwrap(),
            None
        );
        assert_eq!(
            parser.ingest_line(
                r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"signature_delta","signature":"x"}},"session_id":"11111111-2222-4333-8444-555555555555"}"#
            )
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
        assert!(err.to_string().contains(ERR_CLAUDE_PROTOCOL));
        // The error message carries a truncated preview.
        assert!(err.to_string().contains("this is not"));
    }

    #[test]
    fn exit_ok_without_result_is_a_protocol_error() {
        let mut parser = StreamParser::new(SESSION);
        parser.ingest_line(&init_line()).unwrap();
        parser.ingest_line(&text_delta("partial")).unwrap();
        let err = parser.finish(true).unwrap_err();
        assert!(err.to_string().contains(ERR_CLAUDE_PROTOCOL));
    }

    #[test]
    fn non_zero_exit_without_result_reports_missing_result() {
        let mut parser = StreamParser::new(SESSION);
        parser.ingest_line(&init_line()).unwrap();
        let err = parser.finish(false).unwrap_err();
        assert!(err.to_string().contains(ERR_CLAUDE_PROTOCOL));
    }

    #[test]
    fn result_session_mismatch_is_an_error() {
        let mut parser = StreamParser::new(SESSION);
        parser.ingest_line(&init_line()).unwrap();
        let foreign =
            result_line("success", "hi").replace(SESSION, "88888888-8888-4888-8888-888888888888");
        let err = parser.ingest_line(&foreign).unwrap_err();
        assert!(err.to_string().contains(ERR_CLAUDE_SESSION_MISMATCH));
    }

    #[test]
    fn success_outcome_carries_metadata() {
        let mut parser = StreamParser::new(SESSION);
        parser.ingest_line(&init_line()).unwrap();
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
        parser.ingest_line(&init_line()).unwrap();
        parser.ingest_line(&text_delta("streamed")).unwrap();
        // Both final candidates disagree with the stream.
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
        parser.ingest_line(&init_line()).unwrap();
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
