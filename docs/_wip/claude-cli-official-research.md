# Claude Code CLI integration research for Nest Step 1

Status: researched against first-party Anthropic documentation and repositories on 2026-08-22.

Scope: the minimum supported surface for launching Claude Code CLI behind Nest's existing chat UI. This note separates documented contracts from gaps that Nest must not guess around.

## Recommended Step 1 integration shape

Use one `claude -p` subprocess per Nest user turn. Pass the prompt as a process argument or plain-text stdin, request `--output-format stream-json --verbose --include-partial-messages`, capture the Claude `session_id`, and pass `--resume <session-id>` on the next Nest turn. Do not use `--continue`: it selects the most recent session for a working directory and is therefore unsafe when Nest has multiple conversations. Claude documents `-p` as non-interactive mode, `stream-json` as newline-delimited output, and resume-by-ID as the way to continue a specific conversation. [Programmatic usage](https://code.claude.com/docs/en/headless), [CLI reference](https://code.claude.com/docs/en/cli-usage), [sessions](https://code.claude.com/docs/en/agent-sdk/sessions)

For Step 1, start Claude with no model-invocable tools (`--tools ""`) and pass Nest's already-retrieved context in the prompt. This keeps Claude from bypassing Nest's staged-write, diff, approval, pack-permission, symlink, and open-file protections. A later step can expose Nest-owned tools through a controlled protocol such as MCP. `--allowedTools` is not an availability allowlist: it only auto-approves named tools; `--tools` restricts which built-ins are visible. [CLI reference](https://code.claude.com/docs/en/cli-usage), [permissions](https://code.claude.com/docs/en/permissions)

Use `--safe-mode` to suppress host/project customizations while retaining normal authentication, model selection, built-in tools, and permissions. Do not use `--bare` for the initial consumer integration unless Nest explicitly requires API-key/cloud-provider authentication: Anthropic recommends bare mode for scripts, but documents that it does not read subscription OAuth credentials or the system keychain. [CLI reference](https://code.claude.com/docs/en/cli-usage), [programmatic usage](https://code.claude.com/docs/en/headless)

Suggested command shape:

```text
claude -p <prompt>
  --safe-mode
  --tools ""
  --output-format stream-json
  --verbose
  --include-partial-messages
  --model <configured-model>
  [--resume <claude-session-id>]
```

Nest should pass arguments directly to the child-process API, never construct a shell command string.

## Verified CLI contracts

### Headless invocation and output

- `claude -p` / `claude --print` runs non-interactively and exits. Success is exit code `0`; failed runs use a non-zero status. Invalid flags are reported on stderr before a run starts, while failures inside a run, including missing authentication, are emitted as the result on stdout. Nest must inspect both the event stream and process status. [Programmatic usage](https://code.claude.com/docs/en/headless)
- `--output-format` accepts `text`, `json`, and `stream-json`. `stream-json` is newline-delimited JSON. With `--verbose --include-partial-messages`, raw API streaming events are included, enabling token streaming. The final normal line is a `result` message containing response text and session/cost metadata. [Programmatic usage](https://code.claude.com/docs/en/headless)
- Partial token events have outer `type: "stream_event"`; text arrives in raw API `content_block_delta` events whose `delta.type` is `text_delta` and whose text is `delta.text`. Full assistant messages and the final result are still emitted. [Streaming output](https://code.claude.com/docs/en/agent-sdk/streaming-output)
- Core output messages that an adapter must tolerate are `system/init`, `assistant`, `user` tool-result/replay messages, `stream_event`, and `result`. New observability message variants exist and may be added. The init message carries at least the session ID, model, tool list, working directory, permission mode, Claude Code version, and loaded MCP/plugin metadata. Consumers should ignore unknown event types and unknown fields. [TypeScript Agent SDK reference](https://code.claude.com/docs/en/agent-sdk/typescript), [agent loop](https://code.claude.com/docs/en/agent-sdk/agent-loop)
- `system/init.capabilities`, available in Claude Code v2.1.205+, is the documented feature-detection mechanism for protocol behavior. Nest should not branch solely on version strings. [Programmatic usage](https://code.claude.com/docs/en/headless)
- A `result` subtype is one of at least `success`, `error_max_turns`, `error_max_budget_usd`, `error_during_execution`, or `error_max_structured_output_retries`. Only `success` guarantees a final `result` string. All variants carry a session ID and execution metadata, although cost/usage can be absent on some Python SDK error paths. [Agent loop](https://code.claude.com/docs/en/agent-sdk/agent-loop)
- API retries emit `system/api_retry` events with attempt, delay, HTTP status, error category, UUID, and session ID. These are progress signals, not terminal failures. [Programmatic usage](https://code.claude.com/docs/en/headless)

### Input formats

- Plain prompt arguments and piped stdin are documented. Piped input is capped at 10 MB and exceeding it yields a clear non-zero failure. [Programmatic usage](https://code.claude.com/docs/en/headless)
- The CLI reference documents `--input-format text|stream-json` and `--replay-user-messages`; the latter requires stream-json on both input and output. [CLI reference](https://code.claude.com/docs/en/cli-usage)
- Anthropic's Agent SDK streaming-input examples use user messages shaped as:

  ```json
  {
    "type": "user",
    "message": { "role": "user", "content": "..." },
    "parent_tool_use_id": null
  }
  ```

  Content may also be an array of text/image blocks. Streaming input supports queued messages, interruption, and a persistent multi-turn process. [Streaming input](https://code.claude.com/docs/en/agent-sdk/streaming-vs-single-mode)

### Tools and permissions

- `--tools` controls which built-in tools Claude can see; `""` disables them and `default` exposes the normal set. It does not remove MCP tools, which require `--disallowedTools "mcp__*"` when MCP configurations may load. [CLI reference](https://code.claude.com/docs/en/cli-usage)
- `--allowedTools` adds auto-approval rules. It does not make all unlisted tools unavailable. `--disallowedTools` with a bare tool name removes that tool from context; a scoped rule such as `Bash(rm *)` leaves the tool visible but denies matching calls. Permission precedence is deny, then ask, then allow. [CLI reference](https://code.claude.com/docs/en/cli-usage), [permissions](https://code.claude.com/docs/en/permissions)
- For unattended restricted execution, `--permission-mode dontAsk` denies anything that would prompt, but still permits explicit allow rules and Claude's built-in read-only shell command set. `acceptEdits` automatically permits file editing and common filesystem commands. `bypassPermissions` disables the normal checks and is explicitly intended only for isolated environments. [Permission modes](https://code.claude.com/docs/en/permission-modes)
- Read/Edit deny rules do not reliably constrain arbitrary subprocess code: a Python or Node program invoked through a shell can access files outside those built-in tool checks. OS-level sandboxing is the boundary for subprocess access. [Permissions](https://code.claude.com/docs/en/permissions)
- Claude's sandbox covers Bash and child processes, not built-in Read/Edit/Write tools. It is supported on macOS, Linux, and WSL2, but not native Windows. The default sandbox permits writes under the working directory and the session temp directory, and reads much more broadly unless configured otherwise. [Sandboxing](https://code.claude.com/docs/en/sandboxing)

### Working directory and session persistence

- Claude starts with access to the directory in which it is launched. `--add-dir` extends readable/editable working directories. [Permissions](https://code.claude.com/docs/en/permissions)
- Sessions persist conversation history, including prompts, tool calls/results, and responses; they do not snapshot filesystem state. Sessions are written to disk automatically. [Agent SDK sessions](https://code.claude.com/docs/en/agent-sdk/sessions)
- `--continue` selects the most recent conversation associated with the current directory. `--resume <id-or-name>` selects a specific session. `--fork-session` with resume/continue creates a new session ID while retaining copied history. `--session-id` requires a valid UUID. [CLI reference](https://code.claude.com/docs/en/cli-usage)
- A session ID is present in `system/init` and every final result, including error results. Sessions made by `-p` do not appear in the interactive picker but remain resumable by ID. [Agent SDK sessions](https://code.claude.com/docs/en/agent-sdk/sessions), [CLI sessions](https://code.claude.com/docs/en/sessions)
- Anthropic's SDK session guide warns that a mismatched `cwd` is a common cause of resume creating or finding the wrong history, and that the transcript must exist on the same machine. Nest should use one stable, canonical working directory for all turns of a mapped Claude session and persist the Claude session ID alongside the Nest conversation. [Agent SDK sessions](https://code.claude.com/docs/en/agent-sdk/sessions)

### Model selection

- `--model` accepts the current aliases `sonnet`, `opus`, `haiku`, and `fable`, or a full model identifier. It overrides the `model` setting and `ANTHROPIC_MODEL`. `--fallback-model` accepts an ordered comma-separated fallback chain. Unsupported/unavailable models can surface as `model_not_found`. [CLI reference](https://code.claude.com/docs/en/cli-usage), [TypeScript Agent SDK reference](https://code.claude.com/docs/en/agent-sdk/typescript)
- Model aliases intentionally move. Nest should store the user's configured string without hard-coding a permanent list, validate it through a real test invocation, and display the model reported by `system/init` as the effective model.

### Authentication

- Claude Code supports subscription OAuth login, Console/API credentials, Bedrock, Vertex, Foundry, gateways, and credential helpers. On Windows, stored login credentials live under `%USERPROFILE%\.claude\.credentials.json` unless `CLAUDE_CONFIG_DIR` is set. Nest should never read or copy this credential file; it should launch the CLI as the signed-in OS user. [Authentication](https://code.claude.com/docs/en/authentication)
- In non-interactive `-p` mode, `ANTHROPIC_API_KEY` is always used when present. Credential precedence includes cloud-provider selection, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_API_KEY`, `apiKeyHelper`, `CLAUDE_CODE_OAUTH_TOKEN`, and finally stored subscription login. Inheriting a stale API-key environment variable can therefore override a valid subscription login. [Authentication](https://code.claude.com/docs/en/authentication)
- First login is interactive and may open a browser. `/login` is not available inside `-p`. Nest's connection test must distinguish “CLI installed” (`claude --version`) from “authenticated and inference works” (a minimal `-p` request). If authentication is missing, direct the user to run `claude`/`/login` in a terminal rather than trying to conduct OAuth inside the hidden subprocess. [Authentication](https://code.claude.com/docs/en/authentication), [programmatic usage](https://code.claude.com/docs/en/headless)
- Native installer binaries are available for macOS, Linux, WSL, and Windows. The recommended Windows location is `%USERPROFILE%\.local\bin\claude.exe`; `where.exe claude`, `Get-Command claude`, and `claude --version` are the documented discovery/verification tools. [Installation troubleshooting](https://code.claude.com/docs/en/troubleshoot-install)

### Cancellation and process lifetime

- On Unix-like systems, SIGTERM causes `claude -p` to terminate active command process trees, run `SessionEnd` hooks, and exit `143`. It leaves the current turn unfinished and records no result; resuming continues that unfinished turn. SIGINT or the Agent SDK `interrupt()` ends the turn instead. [Programmatic usage](https://code.claude.com/docs/en/headless)
- The SDK's real-time `interrupt()` is available only with streaming input. An interrupted run ends with an `error_during_execution` result, and already-buffered events must be drained before another query. [Streaming input](https://code.claude.com/docs/en/agent-sdk/streaming-vs-single-mode), [Python Agent SDK reference](https://code.claude.com/docs/en/agent-sdk/python)
- If Claude launched background Bash work during `-p`, that process is normally terminated about five seconds after the final result and stdin closes. Background subagent/workflow waiting has a documented default cap of ten minutes. [Programmatic usage](https://code.claude.com/docs/en/headless)

## Ambiguities and unstable surfaces

1. **The direct CLI's stream-json stdin wire protocol is not fully specified.** The CLI reference names the option but does not define all valid message/control variants, framing, malformed-line behavior, permission responses, or compatibility guarantees. The Agent SDK examples verify the user-message shape above, but they are an SDK contract, not an explicit promise that applications should implement the private control protocol themselves. Step 1 should therefore use one-shot input plus resume, not a long-lived hand-rolled bidirectional stream.
2. **The full stream-json output union evolves quickly.** Official SDK types include many lifecycle/observability events beyond the five core messages. Nest should parse a small stable subset, preserve raw diagnostic data where useful, and ignore unknown variants rather than fail the run.
3. **Windows graceful cancellation is not documented equivalently to POSIX signals.** The official SIGTERM behavior does not establish what Rust/Tokio `Child::kill` or Windows `TerminateProcess` preserves. Treat forced termination on Windows as an ungraceful cancel, kill the child process tree, and do not automatically resume that Claude session until an integration test confirms transcript state.
4. **Permission prompts in plain `-p` are not a UI protocol.** Anthropic provides `--permission-prompt-tool` for routing non-interactive permission requests through MCP, and SDK callbacks for full control. Step 1 should prevent prompts by disabling tools; it should not scrape terminal prompt text.
5. **Version-specific behavior matters.** Current docs describe protocol capabilities and fixes introduced across v2.1.x. Nest should record `claude --version`, enforce a tested minimum version, feature-detect through init capabilities where available, and include the CLI version in error diagnostics.

## Concrete implications for Nest Step 1

- Add a backend-owned mapping `Nest chat session -> Claude session ID`; never use `--continue` for routing.
- Spawn one process per user turn with a stable canonical `cwd`, explicit argv, piped stdout/stderr, and an application-level timeout.
- Use `--safe-mode --tools "" --disallowedTools "mcp__*"` and supply Nest RAG/focus context in the prompt. This is the only Step 1 configuration that preserves Nest's current file safety model without implementing a permission/MCP bridge.
- Treat `system/init` as handshake metadata, `stream_event` text deltas as UI tokens, full `assistant` messages as reconciliation data, and `result` as the terminal semantic outcome. Do not mark success solely because the process exited `0`, and do not discard a structured error result solely because the exit code is non-zero.
- Capture stderr separately for setup/argument failures and diagnostics; cap its retained size and redact secrets before persistence or UI display.
- On cancel, stop emitting tokens, terminate the whole process tree, and mark the Nest run cancelled. A graceful cross-platform interrupt and long-lived process are explicitly deferred until Nest adopts the supported Agent SDK/control surface or implements and tests the stream-json control protocol.
- Connection testing should be two-stage: executable/version probe, then a minimal authenticated inference with all tools disabled. Return actionable categories: executable missing, unsupported version, authentication required/expired, model unavailable, network/rate/billing failure, malformed protocol, and process crash.
- Pin a minimum tested Claude Code version in the implementation and CI fixtures. The official documentation itself records behavior changes through at least v2.1.234, so accepting arbitrary old installations without compatibility gates is not supportable.

