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
