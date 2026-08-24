# Claude Agent Step 1 详细设计：Claude CLI 接入与对话入口接管

> 状态：可进入开发  
> 平台：仅 Windows  
> 文档范围：仅 Step 1；Step 2 另行设计  
> 本文是 Claude Agent Step 1 的权威实现依据；早期设置页草案仅保留为 Step 2 输入。  
> 关联研究：[Claude CLI 官方资料调研](./claude-cli-official-research.md)  
> 术语基线：[桌面端领域上下文](../../apps/desktop/CONTEXT.md)

## 1. 目标

在不移除 Nest Agent 的前提下，为桌面端增加一个 Claude Agent 后端。用户可以在 Settings 中配置、检测并连接本机 Claude CLI；当 Claude Agent 已启用且连接可用时，新对话在第一次发送消息时绑定 Claude，并在现有聊天区域中连续与 Claude 闲聊。

Step 1 的交付目标是打通以下最短闭环：

1. 用户配置 Claude CLI 路径并启用 Claude Agent。
2. Nest 自动检测或验证 CLI，显示 CLI 版本和当前有效模型。
3. 用户在现有聊天框中新建会话并发送第一条消息。
4. 该 Nest 会话绑定一个 Claude 会话，流式展示回复。
5. 后续消息通过同一 Claude 会话继续对话。
6. 关闭 Claude Agent 后，新对话继续使用现有 Nest Agent；已绑定会话不切换后端。

“接管原对话入口”是条件性的：Claude Agent 启用且可用时，尚未绑定后端的对话由 Claude 接管；Claude Agent 关闭时，Nest 原有对话能力照常工作。Step 1 不在聊天框提供 Agent 选择器，Step 2 再增加“已启用 Agent → 模型”的二级选择能力。

## 2. 已确认的产品约束

### 2.1 Step 1 包含

- 仅支持原生 Windows。
- Settings 中增加独立 Claude Agent 配置区。
- 支持手动输入 CLI 路径和自动检测。
- 支持测试连接，并显示解析后的 CLI 路径、CLI 版本、当前有效模型和最近测试时间。
- 支持显式 `Save and connect`；若用户未提前检测或测试，保存时自动完成检测和连接测试。
- 支持 Claude Agent 启用开关。
- 提供 Custom models 多行输入框，但仅保存数据，Step 1 运行时不使用。
- 复用现有聊天区域、消息列表、停止按钮和流式事件通道。
- Ask/Agent 两个入口均保留；Claude 后端对两种模式不作逻辑区分。
- 新会话第一次发送时才确定后端并建立 Claude 会话关联。
- Claude 会话使用 Nest 现有 `chat_sessions.id`，不另造一套会话 ID。
- 每轮启动一个 Claude CLI 子进程，通过 `--resume` 保持连续会话。
- Claude 对话路径不生成自动标题，保留 `New chat`，用户仍可手动重命名。

### 2.2 Step 1 不包含

- 不在聊天框提供 Agent 或模型选择。
- 不让 Claude 通过 Nest 的 RAG、知识库索引、引用、提案或审批能力工作。
- 不封装 Nest 工具给 Claude。
- 不实现 Claude 工具权限、风险分级或沙箱策略。
- 不限制 Claude 原生工具、插件、MCP 或用户级 Claude 配置。
- 不增加环境变量配置 UI。
- 不指定或覆盖 Claude system prompt。
- 不传递 `--model`；Custom models 不参与 Step 1 执行。
- 不实现 Claude 会话自动标题。
- 不迁移或删除 Claude CLI 自己保存的 transcript。
- 不支持 macOS、Linux、WSL 或远程 Claude CLI。
- 不设置最低 Claude CLI 版本门槛。
- 不在 resume 失败时自动回放历史或创建替代 Claude 会话。

### 2.3 明确接受的 Step 1 风险

Claude CLI 将以当前 Vault 根目录作为工作目录启动，并继承用户的 Claude 配置。Step 1 不禁用 Claude 原生工具，因此 Claude 可能直接读取或修改 Vault 文件，绕过 Nest 的提案、审批、保护路径和索引更新机制。

这是本地开发阶段明确接受的限制，不应在 UI 或文档中暗示 Claude 路径享有 Nest Agent 的文件安全保证。Step 2 设计 Nest 能力封装和风险工具接入时必须重新审视该边界。

## 3. 当前实现与扩展接缝

当前桌面端聊天链路为：

```text
ChatPanel / MentionComposer
        │ invoke chat_send + listen chat://stream
        ▼
commands/chat.rs
        │ 持久化用户消息、组装历史、处理停止与标题
        ▼
agent.rs
        │ Rig + OpenAI-compatible API + RAG + Nest Agent tools
        ▼
chat_events.rs ──► Token / Thinking / Sources / FileChanged / Done / Error
```

主要现状：

- `commands/chat.rs::chat_send` 同时承担用例编排和 Nest Agent 具体调用，尚无聊天后端分派层。
- `agent.rs::run_agent_chat` 是现有 Nest Agent 实现，包含 LLM 客户端、RAG、Ask/Agent 差异、工具和流式处理。
- `chat_events.rs::ChatStreamEvent` 已基本与提供方无关，可复用于 Claude 文本和思考流。
- `db.rs::chat_sessions` 当前没有后端归属字段。
- Settings 使用全局延迟自动保存；Claude 配置要求显式副作用，不能直接沿用该保存时机。
- `ChatSessionBar` 目前先创建空白 Nest 会话；该行为保留，后端延迟到第一条消息绑定。

本设计增加一层窄而深的 Chat Runtime：`commands/chat.rs` 只处理通用对话用例，Runtime 隐藏 Nest/Claude 的选择和提供方细节。现有 `agent.rs` 继续作为 Nest Agent adapter，不在 Step 1 中重写。

```text
                         ┌──────────────────────┐
SettingsPanel ─────────►│ Claude configuration │
                         │ detect/test/save     │
                         └──────────┬───────────┘
                                    │
ChatPanel ─► chat_send ─► Chat Runtime ─┬─► Nest adapter (`agent.rs`)
      ▲             │                  │
      │             │                  └─► Claude adapter (`claude_cli.rs`)
      │             │                         │ one process / turn
      └─ stream events ◄──────────────────────┴─► local Claude CLI
```

## 4. 领域模型

### 4.1 Chat Backend

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatBackend {
    Nest,
    Claude,
}
```

`ChatBackend` 是一个聊天会话不可变的执行归属。它不是“当前全局开关值”，也不是每条消息的临时参数。

### 4.2 Backend Binding

- 新创建的空白会话：`backend = NULL`，称为 unbound。
- 第一次发送消息时：根据当时的 Claude 全局配置原子地绑定 `nest` 或 `claude`。
- 已绑定会话：后续始终使用同一后端，不因 Settings 改动而变化。
- 切换 Claude 开关不会创建新会话，也不会改写当前会话。

绑定规则：

| 第一次发送时的状态 | 结果 |
|---|---|
| Claude 未启用 | 绑定 `nest` 并走现有 Nest Agent |
| Claude 已启用且配置可用 | 绑定 `claude` 并启动 Claude 首轮 |
| Claude 已启用但配置不可用 | 阻止发送；保持 `backend = NULL`，不回退 Nest |

### 4.3 Claude Session Binding

- `chat_sessions.id` 当前是 UUID v4，直接作为 Claude `session_id`。
- Claude 首轮使用 `--session-id <chat_session.id>`。
- Claude 后续轮次使用 `--resume <chat_session.id>`。
- 不新增 `claude_session_id` 字段，避免两个 ID 的同步问题。
- Nest 删除会话时只删除 Nest 数据，不删除 Claude CLI 的外部 transcript。

### 4.4 Backend 状态

Claude-bound 会话需要持久化最少运行状态：

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatBackendStatus {
    Uninitialized,
    Ready,
    Unresumable,
}
```

- `uninitialized`：Claude backend 已绑定，但 Nest 还没有收到匹配的 `system/init`。它只表示 Nest 未确认状态，**不证明 Claude transcript 不存在**；下一次先使用 `--session-id`，若 CLI 报 ID 已存在则按 §12.2 对账并透明 resume 一次。
- `ready`：Nest backend 已就绪，或 Claude session 已经收到匹配的 init；Claude 后续使用 `--resume`。
- `unresumable`：Claude 明确报告无法恢复该 session；会话进入只读状态。

普通的单轮错误、取消和当前连接不可用不把会话永久标记为 `unresumable`。`unresumable` 只用于明确的 resume 失败，防止悄悄生成一个语义上不同的新 Claude 会话。该三态还解决首轮在 init 前失败的重试问题：不能仅根据“是否已有用户消息”决定使用 `--session-id` 还是 `--resume`。

## 5. 数据设计

### 5.1 Settings

在 Rust `AppSettings` 和 TypeScript `AppSettings` 增加：

```rust
#[serde(default)]
pub claude_agent_enabled: bool,
#[serde(default)]
pub claude_cli_path: String,
#[serde(default)]
pub claude_custom_models: String,
```

三个字段分别使用 `serde(default)`，使不含 Claude 字段的 `GeneralSettingsUpdate` 仍可被现有 `settings_set(AppSettings)` 反序列化；安全边界仍是后端持久化白名单，而不是反序列化默认值。

默认值：

```text
claude_agent_enabled = false
claude_cli_path = ""
claude_custom_models = ""
```

Custom models 保存前规范化：逐行 trim、移除空行、按精确字符串去重并保持首次出现顺序。Step 1 不校验模型是否存在，也不传给 CLI。

连接报告不属于用户可编辑 Settings，不应混入 `AppSettings` 往返覆盖。新增只读模型：

```rust
pub struct ClaudeConnectionReport {
    pub status: ClaudeConnectionStatus,
    pub configured_cli_path: String,
    pub resolved_cli_path: String,
    pub cli_version: String,
    pub effective_model: String,
    pub tested_at: String,
    pub message: Option<String>,
}
```

```rust
pub enum ClaudeConnectionStatus {
    Disabled,
    Connected,
    LastConnected,
    Unavailable,
}
```

持久化“最近一次成功报告”，建议以单独 settings key 的 JSON 保存，例如 `claude_connection_report_v1`。报告必须包含当次测试对应的 `configured_cli_path`；只有当前路径与报告匹配时，重启后才能展示 `last_connected` 并允许尝试聊天。路径变更会使旧报告失效。

运行期错误可以把内存状态切换为 `unavailable`，并更新报告中的状态/消息；只有成功测试才能写入新的版本、模型和时间。

### 5.2 ChatSession

在 Rust/TypeScript `ChatSession` 增加：

```rust
pub backend: Option<ChatBackend>,
pub backend_status: ChatBackendStatus,
```

SQLite schema 增加：

```sql
ALTER TABLE chat_sessions ADD COLUMN backend TEXT;
ALTER TABLE chat_sessions
    ADD COLUMN backend_status TEXT NOT NULL DEFAULT 'uninitialized';
```

迁移策略：

1. 老版本已有会话全部迁移为 `backend = 'nest'`。
2. 已有会话的 `backend_status` 迁移为 `ready`。
3. 升级后新建会话明确写入 `backend = NULL`、`backend_status = 'uninitialized'`。
4. 首次绑定 Nest 时同时把 status 写为 `ready`；首次绑定 Claude 时保持 `uninitialized`。
5. Claude 收到 session ID 匹配的 `system/init` 后立即把 status 写为 `ready`，即使后续本轮失败或取消也应尝试 resume。
6. 读取时遇到未知 backend/status 值必须返回可诊断错误，不能自动当作 Nest。

现有开发态会话不要求保留旧行为差异；统一归属 Nest 是最简单且可预测的迁移。

### 5.3 原子绑定

第一次发送必须在一个数据库事务中完成：

1. 读取 session，并确认存在。
2. 若 `backend IS NULL`，计算目标后端。
3. 使用条件更新 `WHERE id = ? AND backend IS NULL` 写入后端。
4. 若条件更新未命中，重新读取，以并发请求先绑定的值为准。
5. 使用最终 backend 插入用户消息。
6. 提交事务后才启动后端执行。

“后端绑定”和“第一条用户消息插入”不可分裂，否则快速双击、重复 invoke 或窗口竞争可能产生一个会话中两种后端的消息。

建议数据库接口：

```rust
pub struct PreparedChatTurn {
    pub session: ChatSession,
    pub user_message: ChatMessage,
}

pub fn bind_backend_and_insert_user_message(
    conn: &mut Connection,
    session_id: &str,
    requested_backend: ChatBackend,
    content: &str,
) -> AppResult<PreparedChatTurn>;
```

当 Claude 已启用但不可用时，在事务之前拒绝发送，不绑定 backend，也不插入用户消息。

## 6. 后端模块设计

### 6.1 Chat Runtime

新增 `src-tauri/src/chat_runtime.rs`，作为唯一聊天后端分派入口：

```rust
pub struct ChatRunRequest<'a> {
    pub app: &'a AppHandle,
    pub state: SharedState,
    pub session: &'a ChatSession,
    pub query: &'a str,
    pub history: Vec<ChatMessage>,
    pub focus_paths: Vec<String>,
    pub mode: ChatMode,
    pub protected_paths: Vec<String>,
}

pub struct ChatRunResult {
    pub answer: String,
    pub citations: Vec<Citation>,
    pub thinking: String,
    pub thinking_seconds: Option<f64>,
    pub file_changes: Vec<FileChange>,
}

pub async fn run_chat(request: ChatRunRequest<'_>) -> AppResult<ChatRunResult>;
```

职责：

- 根据已绑定的 `session.backend` 分派。
- Nest：转换为现有 `agent::AgentChatRequest`，保持行为不变。
- Claude：构造 `ClaudeTurnRequest`，不携带 RAG、focus paths、protected paths 或 Nest 历史。
- 对调用方返回统一结果，事件仍通过现有 `chat://stream` 发出。
- unbound session 到达此层视为编程错误；绑定必须先在 command/use-case 层完成。

`commands/chat.rs` 保留通用职责：参数校验、取消前一任务、原子准备 turn、消息持久化、统一 Done/Error、Nest 标题触发条件。它不再直接依赖 Rig 或 Claude NDJSON。

### 6.2 Claude CLI Adapter

新增 `src-tauri/src/claude_cli.rs`，对上层隐藏 Windows 可执行解析、子进程、NDJSON 和 Claude 事件差异。

建议公开接口：

```rust
pub struct ClaudeCliConfig {
    pub configured_path: String,
}

pub enum ClaudeLaunchTarget {
    Executable { executable: PathBuf },
    NodeScript { node_executable: PathBuf, script: PathBuf },
}

pub struct ClaudeDetection {
    pub configured_path: String,
    pub resolved_path: String,
    pub launch_target: ClaudeLaunchTarget,
}

pub struct ClaudeProbeResult {
    pub resolved_path: String,
    pub cli_version: String,
    pub effective_model: String,
}

pub struct ClaudeTurnRequest<'a> {
    pub vault_root: &'a Path,
    pub session_id: &'a str,
    pub is_first_turn: bool,
    pub prompt: &'a str,
}

pub struct ClaudeTurnResult {
    pub answer: String,
    pub thinking: String,
    pub cli_session_id: String,
    pub model: Option<String>,
}

pub fn detect_cli(configured_path: Option<&Path>) -> AppResult<ClaudeDetection>;
pub async fn probe_cli(detection: &ClaudeDetection) -> AppResult<ClaudeProbeResult>;
pub async fn run_turn(
    detection: &ClaudeDetection,
    request: ClaudeTurnRequest<'_>,
    event_sink: impl Fn(ChatStreamEvent) + Send,
) -> AppResult<ClaudeTurnResult>;
```

内部可再分成 resolver/parser/process，但不为每个小步骤暴露 Tauri command。

### 6.3 SharedState

代码库对外传递的是 `SharedState = Arc<AppState>`。Claude 运行态字段加在内部 `AppState` 上，commands 和 runtime 继续接收/借用 `SharedState`，与现有命名和所有权模型保持一致：

```rust
pub struct ClaudeRuntimeState {
    pub report: RwLock<ClaudeConnectionReport>,
    pub active_process: Mutex<Option<ClaudeProcessHandle>>,
}
```

现有 `state.rs` 只允许一个全局聊天任务，因此继续复用其取消通道；`ClaudeProcessHandle` 只保存终止 Windows 进程树需要的信息，不承担连接池职责。Step 1 每轮一个进程，不常驻 Claude 进程。

## 7. Windows CLI 解析

### 7.1 支持的启动形态

1. 原生 `claude.exe`：直接启动。
2. npm 安装：使用 `node.exe <.../claude-code/cli-wrapper.cjs>` 启动。

Windows 上 `Get-Command claude` 常返回 `claude.ps1` 或 `claude.cmd`。Tauri 不应通过 PowerShell/cmd shell 执行这些 shim；解析器需要定位它们指向的 npm 包和 `cli-wrapper.cjs`，再查找配套 `node.exe`。这样避免 shell quoting、执行策略和进程树不稳定。

### 7.2 手动路径规则

- 空路径：允许 Auto-detect 或 Save and connect 自动检测。
- `claude.exe`：验证文件存在并可执行。
- `cli-wrapper.cjs`：查找 Node 后作为 NodeScript 启动。
- `claude.cmd` / `claude.ps1` / 无扩展名 npm shim：只用于解析其真实 wrapper；不直接执行。
- 目录：尝试目录内的 `claude.exe`，然后尝试 npm wrapper 结构。
- 其他文件：返回 `invalid_cli_path`，不猜测执行。

路径保存用户输入的规范化绝对路径；连接报告保存真正启动的 resolved path。UI 同时展示二者有助于解释“输入 shim，实际执行 wrapper”。

### 7.3 自动检测顺序

1. PATH 上的原生 `claude.exe`。
2. PATH 上 `claude` 命令解析出的 npm shim，再定位 `cli-wrapper.cjs`。
3. 常见 npm global 位置中的 wrapper。

找到候选后必须运行版本探测，不能仅凭文件存在判定成功。若多个候选，第一个探测成功者胜出；检测结果只更新前端 draft，不自动保存。

## 8. 连接探测与 Settings 交互

### 8.1 Tauri Commands

建议新增：

```rust
claude_detect_cli(configured_path: Option<String>) -> ClaudeDetectionDto
claude_test_connection(configured_path: String) -> ClaudeConnectionReport
claude_save_settings(request: ClaudeSettingsRequest) -> ClaudeConnectionReport
claude_connection_status() -> ClaudeConnectionReport
```

`claude_save_settings` 是显式事务边界：

```rust
pub struct ClaudeSettingsRequest {
    pub enabled: bool,
    pub cli_path: String,
    pub custom_models: String,
}
```

- `enabled = true`：持久化规范化配置，然后自动 resolve、版本探测和最小连接测试；成功写入 last-success report，失败返回 unavailable 并保留已启用配置。
- `enabled = false`：只保存，返回 disabled，不启动 CLI。
- CLI 路径或配置改变时，旧 last-success report 不再作为当前配置的成功证明。

不要让通用 `settings_set` 的 450 ms autosave 承担上述行为。隔离必须同时存在于两层：

1. 前端把 `settingsSet` 参数类型收窄为 `GeneralSettingsUpdate = Omit<AppSettings, "claude_agent_enabled" | "claude_cli_path" | "claude_custom_models">`，从 autosave payload 中剥离这三个字段，并在合并通用 settings 响应时保留 Claude draft。
2. Rust `settings_set` 必须使用非 Claude 字段白名单持久化，**绝不写入上述三个 Claude 字段**；不得依赖前端剥离保证事务边界。建议新增 `db::save_general_settings`，只更新既有通用 keys，而 `claude_save_settings` 独占三个 Claude keys 和连接报告的写入权。

后端采用写入白名单，不采用“先读完整设置、覆盖三个字段、再全量保存”的 read-modify-write，因为它可能与并发的 `claude_save_settings` 产生丢失更新。这样即使旧前端或竞态把完整 form 发送给 `settings_set`，也无法绕过 `Save and connect`。

### 8.2 按钮语义

| 操作 | 修改 draft | 持久化 | 运行测试 |
|---|---:|---:|---:|
| Auto-detect | 是，填入检测路径 | 否 | 仅候选版本探测 |
| Test connection | 是，可展示临时报告 | 否 | 是 |
| Save and connect（enabled） | 是 | 是 | 是，未提前测试也必须执行 |
| Save（disabled） | 是 | 是 | 否 |

测试连接至少执行：

1. 解析启动目标。
2. 执行版本命令并读取 CLI 版本。
3. 运行一个最小的 headless `-p` 请求，使用 stream-json 和 [`--no-session-persistence`](https://code.claude.com/docs/en/cli-usage)，避免连接测试留下可恢复会话。
4. 从 `system/init` 读取当前有效模型。
5. 等待语义终态 `result`，成功后生成 report。

测试不传 `--model`，因此读取到的是当前 Claude 配置实际生效的默认模型。不得通过 Custom models 推断连接模型。

探测超时建议定义为 adapter 内部常量：版本命令 10 秒，最小连接请求 120 秒。普通聊天不设置人工总时限，由用户 Stop；stdout/stderr reader 和进程退出必须受取消信号控制。

### 8.3 状态展示

| 状态 | UI |
|---|---|
| disabled | “Claude Agent is disabled” |
| connected | 绿色成功提示；展示 resolved path、version、model、tested_at |
| last_connected | 展示 “Last connected”；允许聊天在实际调用时验证 |
| unavailable | 错误说明和重试入口；展示 Settings 链接 |

重启时，如果当前配置与 last-success report 匹配，状态为 `last_connected`，不强制启动时测试。第一次实际发送失败后切换为 `unavailable`。

## 9. 对话发送流程

### 9.1 前端预检

ChatPanel 从 session 和 Claude connection status 派生 composer 状态：

| 会话 | Claude 配置 | Composer |
|---|---|---|
| unbound | disabled | 可发送，首次绑定 Nest |
| unbound | connected/last_connected | 可发送，首次绑定 Claude |
| unbound | unavailable | 禁用发送，提示打开 Settings；不回退 |
| Nest-bound | 任意 | 可发送，继续 Nest |
| Claude-bound | enabled + available | 可发送，继续 Claude |
| Claude-bound | disabled | 只读，提示重新启用 Claude |
| Claude-bound | unavailable | 只读，提示修复连接 |
| Claude-bound + unresumable | 任意 | 永久只读，提示新建对话 |

后端必须重复相同校验，不能依赖前端禁用状态保证正确性。

### 9.2 首轮 Claude

命令形态（展示语义，实际使用 `Command` 参数数组）：

```text
claude -p
  --output-format stream-json
  --verbose
  --include-partial-messages
  --session-id <nest-chat-session-uuid>
```

- `cwd` 设置为当前 Vault 根目录。
- 用户 prompt 通过 child stdin 写入后关闭 stdin，不放入 argv。
- 不传 Nest 历史、RAG 上下文、focus paths、system prompt 或 `--model`。
- 不传 `--safe-mode`、`--tools`、`--disallowedTools` 等限制参数。
- 不禁用用户设置、plugins 或 MCP。

CLI 返回的 `system/init.session_id` 必须等于 Nest session ID；不一致时本轮失败，防止关联错乱。

Claude-bound session 是否为首轮由 `backend_status` 决定，而不是消息数量：`uninitialized` 先使用 `--session-id`，收到匹配 init 后持久化为 `ready`；`ready` 使用 `--resume`。Nest 在 init 前失败时无法判断 Claude 是否已经落盘：同一 UUID 若尚未存在即可继续创建，若返回 `Session ID ... is already in use` 则同轮透明 resume 一次，详见 §12.2。若 init 后失败，下一轮直接恢复已经确认存在的 Claude transcript。

### 9.3 后续 Claude 轮次

```text
claude -p
  --output-format stream-json
  --verbose
  --include-partial-messages
  --resume <nest-chat-session-uuid>
```

仅把本轮用户输入写入 stdin；上下文由 Claude transcript 恢复。禁止使用 `--continue`，因为它按工作目录选择最近会话，无法保证与当前 Nest 会话一一对应。

### 9.4 Nest 路径

Nest-bound 会话继续执行当前逻辑：

- 构造 Nest 历史。
- Ask/Agent 按当前语义区分。
- 保留 RAG、mentions、citations、staged proposals 和 protected paths。
- 保留现有自动标题。

Claude-bound 会话即使前端 mode 为 Agent，也忽略模式差异。建议仍记录消息当时 mode 以兼容现有 schema，但不让它改变 Claude 参数。

## 10. NDJSON 流协议

stdout 按行读取，每行独立解析 JSON。解析器必须对新增字段和未知事件类型前向兼容：已知字段按需读取，未知字段忽略，未知 type 记录 debug 日志后继续。

### 10.1 事件映射

| Claude 事件 | Nest 行为 |
|---|---|
| `system` / `init` | 校验 session ID；记录 model/version 等元数据，不展示为消息 |
| `stream_event` text delta | 发出 `ChatStreamEvent::Token` |
| `stream_event` thinking delta | 发出 `ChatStreamEvent::Thinking` |
| assistant message | 作为无 partial stream 时的文本候选或校验信息 |
| user/tool result | Step 1 不映射 UI；工具可能已执行 |
| `result` success | 作为语义终态，完成本轮并补齐最终文本 |
| `result` error subtype | 映射为分类错误 |
| 未知 type | 忽略并继续 |

### 10.2 文本去重

CLI 可能同时输出 partial delta、assistant 完整消息和 result 文本。适配器维护：

```text
streamed_text
assistant_candidate
result_candidate
```

选择规则：

1. 已收到 text deltas：UI 只增量发 delta，最终 answer 采用完整度最高且以 streamed_text 为前缀/相等的候选；不得再次发完整文本。
2. 未收到 deltas：在收到 assistant/result 候选时只缓存，成功终态时一次性发最终文本。
3. 候选互相冲突：以 `result` 的成功结果为终态依据，记录诊断日志，不拼接重复内容。
4. 进程 exit 0 但没有成功 `result`：视为协议错误，不保存 assistant 消息。

思考内容同样只增量展示一次。Step 1 不要求持久化 Claude 原生 tool-use 卡片。

### 10.3 输出健壮性

- stdout 只接受 UTF-8 NDJSON；单行无法解析时返回包含截断预览的 protocol error，不能把原始 JSON 当普通回复。
- stderr 单独限长收集，用于错误诊断，不混入 assistant content。
- 为 stdout/stderr 设置上限或流式消费，避免子进程阻塞。
- prompt 使用 stdin，遵守 Claude headless 输入限制；超限返回明确错误。
- 未知字段不导致失败，满足无最低版本门槛的兼容策略。

## 11. 消息持久化与标题

通用规则：

1. 用户消息在后端执行前持久化。
2. 只有收到成功 `result` 后才持久化 assistant 消息。
3. 取消或失败时不持久化不完整 assistant 文本；前端可展示本轮临时流和错误。
4. `Done` 只在 assistant 持久化成功后发出。

Claude 首轮失败时，session 已经绑定 Claude 且用户消息已存在。后续重试应继续使用相同 session ID；不得把它改为 Nest。若 CLI 明确表明 session 未创建，仍保留绑定，用户可修复连接后重试。

标题规则：

- `backend = nest`：维持现有自动标题逻辑。
- `backend = claude`：Step 1 永不触发 LLM 标题任务，保持 `New chat`；手动重命名可用。

## 12. 停止、错误与恢复

### 12.1 Stop

Stop 必须终止本轮 Claude 的整个 Windows 进程树，而不只是父进程 pipe。实现应保存 PID，并使用 Windows Job Object 或等价的进程树终止方式；Node wrapper 形态也必须覆盖其子进程。

停止流程：

1. 标记通用 chat cancellation。
2. 关闭 stdin。
3. 终止 Job/process tree。
4. 等待 stdout/stderr reader 退出并回收 child。
5. 发出取消类 Error/终态，清理 active handle。

用户已接受 Step 1 的试用策略：取消后下一轮仍尝试恢复同一 session。若取消发生在匹配 init 之后，直接使用 `--resume`；若发生在 init 之前，status 仍为 `uninitialized`，先使用 `--session-id`，命中“ID 已存在”时按 §12.2 透明改用 `--resume` 一次。由于 Claude 官方未承诺 Windows 强杀后的 transcript 完整性，明确的 resume 不可恢复错误仍按 unresumable 处理。

### 12.2 错误分类

建议内部错误码：

| 错误码 | 含义 | 状态变化 |
|---|---|---|
| `claude_disabled` | Claude-bound 会话但全局已关闭 | 会话只读；不改 binding |
| `claude_unavailable` | 当前配置无可用连接证明或实际启动失败 | runtime/report → unavailable |
| `invalid_cli_path` | 路径无法解析 | unavailable |
| `claude_auth_required` | CLI 未登录或认证失败 | unavailable，提示在终端完成登录 |
| `claude_protocol_error` | NDJSON/终态不符合协议 | 本轮失败，可重试 |
| `claude_process_failed` | 非零退出、spawn/IO 失败 | unavailable 或本轮失败，按原因区分 |
| `claude_session_mismatch` | CLI init ID 与 Nest ID 不一致 | 本轮失败，不保存 assistant |
| `claude_session_id_in_use` | `--session-id` 阶段在 init/result 前快速失败，stderr 匹配 `Error: Session ID ... is already in use.` | 同一轮、同一 ID、同一 prompt 透明改用 `--resume`，最多一次 |
| `claude_session_unresumable` | `--resume` 明确失败 | session.backend_status → unresumable |
| `chat_cancelled` | 用户停止 | 本轮失败；下轮按 status 选择 resume，或通过 ID-in-use 对账进入 resume |

错误消息不得包含完整 prompt、API token、环境变量或未截断 stderr。日志中 session ID 和路径可保留，但 stderr 需长度限制和基础敏感信息遮盖。

`claude_session_id_in_use` 是首轮初始化的对账分支，不是普通重试策略：

1. 仅当本次启动模式为 `--session-id`、尚未收到有效 init/result、进程已失败退出，且去除 ANSI/CRLF 后的 stderr 同时包含 `Session ID` 和 `is already in use` 时触发。
2. 立即以 `--resume <同一 ID>` 重启一次，复用同一 prompt；不向 UI 发出中间 Error，也不重复持久化用户消息。
3. resume 收到匹配 `system/init` 时立刻把 status 更新为 `ready`，之后按正常流继续。
4. 该 resume 若 stderr 包含 `No conversation found`，这是“ID 已占用但 transcript 不可恢复”的矛盾结果，分类为 `claude_protocol_error`，保存受限诊断信息；不得再次回到 `--session-id`，也不得在同一轮循环重试。
5. 透明 fallback 每个用户 turn 最多一次。其他 `--session-id` 错误不触发 resume。

分类器使用上述两段英文作为稳定匹配锚点，但不要求完整字符串全等，以兼容错误中插入具体 UUID 和前后缀。匹配逻辑应有 fixture 测试，避免普通回复或 stderr 中的偶然文本触发。

### 12.3 无回退原则

以下情况均不得自动切到 Nest：

- Save and connect 失败。
- Claude 启用但启动失败。
- 流协议错误。
- 用户停止。
- resume 失败。
- Claude 被关闭而当前会话已绑定 Claude。

用户可修复 Settings 后继续，或新建会话。在 `unresumable` 情况下只能新建会话。

## 13. 前端详细设计

### 13.1 SettingsPanel

Claude Agent section 字段和布局：

```text
[ Claude Agent toggle ]

CLI path [................................] [Auto-detect]
                                           [Test connection]

Connection status
Resolved path / CLI version / Effective model / Tested at

Custom models (one per line)
[.........................................................]
Used by the agent/model selector planned for Step 2.

[Save and connect]   // enabled
[Save]               // disabled
```

交互要求：

- section 使用独立 local draft 和 dirty 状态。
- 初次加载以持久化配置填充 draft，以 connection status 填充报告。
- Auto-detect/Test 不触发通用 Settings 保存。
- 按钮执行中禁用重复点击并显示明确 loading 文案。
- 保存失败保留 draft，不恢复旧值。
- Test 成功但未保存时，显示“Not saved”提示。
- 路径/开关在成功测试后又被编辑时，临时成功状态标记 stale。
- enabled 打开且保存连接失败时，开关仍保持打开，状态 unavailable。

### 13.2 ChatPanel

- 继续使用现有 composer 和消息流。
- Ask/Agent selector 保留原样；Claude session 中两者效果相同，可在 selector 附近显示简短说明。
- 当前 session 已绑定的 backend 应在 UI 可识别，但 Step 1 不提供可点击切换。
- Settings 改变且当前会话已绑定另一个 backend 时，显示：“设置将应用于新对话”及 `New chat` 按钮。
- Claude disabled/unavailable/unresumable 时禁用 composer，但消息历史、手动重命名和删除仍可用。
- unbound + Claude unavailable 不允许发送，不创建用户消息。
- Claude 原生 tool-use 不做专用 UI；最终文本仍正常显示。
- Claude 流中不显示 Nest Sources、citations 或 file-change proposals。

### 13.3 Query 与缓存

新增 query key，例如 `claudeConnectionStatus`。以下操作成功后更新/失效它：

- Test connection：仅更新局部 draft report，不覆盖全局 persisted query。
- Save and connect：更新全局 status query 和 settings query。
- chat spawn/运行失败：后端返回状态变化后失效 status query。
- toggle 保存：更新 settings/status，并重新计算 composer availability。

Session backend/status 随 session API 返回，绑定或标记 unresumable 后更新 session list/detail cache。

## 14. Tauri 配置与依赖

### 14.1 Tokio

Claude adapter 需要异步 process 和 pipe IO。`tokio` features 至少增加：

```toml
features = ["rt-multi-thread", "macros", "sync", "process", "io-util"]
```

若实现选择 `tokio::time` 做 probe timeout，确保 `time` feature 可直接使用，不依赖间接 feature 偶然开启。

### 14.2 Capability

CLI 仅由 Rust `std::process`/`tokio::process` 启动，不把任意 shell 权限暴露给前端。前端只能调用固定的 Claude commands；path 和 prompt 必须作为结构化参数进入 Rust，不拼接 shell 字符串。

### 14.3 Command 注册

在 `commands/mod.rs` re-export Claude 设置 commands，并在 `lib.rs` 的 invoke handler 注册。Chat runtime/adapter 本身不是 command。

## 15. 文件级实施清单

预计改动如下，开发时以实际结构为准：

| 文件 | 改动 |
|---|---|
| `packages/shared/src/index.ts` | 增加 ChatBackend、ChatBackendStatus、Claude settings/report DTO 和 GeneralSettingsUpdate；扩展 ChatSession |
| `apps/desktop/src-tauri/src/db.rs` | settings 默认值与读写；`save_general_settings` 排除 Claude keys；chat_sessions 迁移；原子绑定+消息插入；backend status 更新 |
| `apps/desktop/src-tauri/src/state.rs` | 在 `AppState` 保存 Claude runtime report 和 active process handle，通过 `SharedState` 访问 |
| `apps/desktop/src-tauri/src/chat_runtime.rs` | 新增统一后端分派 |
| `apps/desktop/src-tauri/src/claude_cli.rs` | 新增 Windows resolve/probe/run/parse/kill 实现 |
| `apps/desktop/src-tauri/src/agent.rs` | 保持 Nest adapter，必要时仅做接口适配 |
| `apps/desktop/src-tauri/src/commands/chat.rs` | 使用 runtime；首次原子绑定；按 backend 控制标题和错误状态 |
| `apps/desktop/src-tauri/src/commands/claude.rs` | Claude detect/test/save/status commands |
| `apps/desktop/src-tauri/src/commands/mod.rs` | re-export commands |
| `apps/desktop/src-tauri/src/lib.rs` | 注册 commands/modules |
| `apps/desktop/src-tauri/Cargo.toml` | Tokio process/io features；如使用 Windows Job Object，增加相应 Win32 依赖 |
| `apps/desktop/src/lib/api.ts` | 新增 Claude settings API；把 `settingsSet` 收窄为 GeneralSettingsUpdate；扩展 session types 的使用 |
| `apps/desktop/src/lib/query-keys.ts` | connection status query key |
| `apps/desktop/src/components/settings/SettingsPanel.tsx` | 独立 Claude draft、按钮和状态报告 |
| `apps/desktop/src/components/chat/ChatPanel.tsx` | backend-aware composer、状态提示、Claude stream 行为 |
| `apps/desktop/src/components/chat/MentionComposer.tsx` | 仅增加 Claude 模式说明（若放在组件内） |
| `apps/desktop/src/components/chat/ChatSessionBar.tsx` | 保持新建空白 session；可增加 backend 标识 |

## 16. 实施顺序

建议按可测试的纵向切片实施：

1. **共享模型与迁移**：增加 backend/settings/report 类型、数据库迁移和原子绑定测试。
2. **CLI resolver**：在 Windows fixture/临时目录测试 exe、cmd/ps1 shim、wrapper 和无效路径。
3. **NDJSON parser**：用固定事件样本测试 init、delta、result、未知字段、重复文本和错误终态。
4. **Process adapter**：实现 probe、stdin prompt、每轮 process、cwd 和取消；以 fake CLI fixture 做集成测试。
5. **Chat Runtime**：把现有 Nest 调用移入分派，先证明 Nest 回归不变，再接 Claude。
6. **Settings commands/UI**：完成 detect/test/save/status 和 autosave 隔离。
7. **Chat UI**：完成首次绑定、禁用状态、应用于新会话提示和 Claude 流。
8. **真实 CLI 验收**：在 Windows 已登录 Claude CLI 上完成连续会话和停止测试。

每一步先补对应测试，再实现最小代码通过；不要等所有 UI 完成后才验证 CLI 协议。

## 17. 测试设计

### 17.1 Rust 单元测试

**Database**

- 旧 schema 迁移后已有 session 为 Nest。
- 新 session backend 为 NULL、status 为 uninitialized。
- 第一次绑定 Nest/Claude 成功。
- Nest 首次绑定同时变为 ready；Claude 在匹配 init 后变为 ready。
- Claude 首轮在 init 前失败时仍使用 `--session-id`，init 后失败时改用 `--resume`。
- 即使完整 AppSettings 直接传给 `settings_set`，三个 Claude 字段也不落库，通用字段仍正常保存。
- `settings_set` 与 `claude_save_settings` 并发时不发生 Claude 配置丢失更新。
- 重复绑定不覆盖已有 backend。
- 并发式条件更新只产生一个 backend。
- backend 绑定与用户消息写入同时提交/回滚。
- unresumable 状态可持久化。

**Resolver**

- 手动 `claude.exe`。
- 手动 `cli-wrapper.cjs` + Node。
- `.cmd`/`.ps1`/extensionless shim 解析到底层 wrapper。
- 目录输入。
- 相对路径规范化。
- 文件不存在、非支持文件、Node 缺失。
- 路径含空格。

**Parser**

- init 提取 session/model/version。
- text/thinking delta 正确映射。
- partial + final 不重复。
- 无 partial 时 final 一次性输出。
- result error subtype。
- 未知 type/字段被忽略。
- 非 JSON 行、缺少 result、session mismatch。

**Runtime**

- Nest session 只调用 Nest adapter。
- Claude session 只调用 Claude adapter。
- Claude Ask/Agent 参数等价。
- Claude 不获得 focus/RAG/history/protected paths。
- Claude 成功不触发自动标题。
- Nest 行为和标题保持原状。

### 17.2 Fake CLI 集成测试

提供测试专用可执行/Node script fixture，从 stdin 读取 prompt 并输出可控 NDJSON：

- probe 返回固定 version/model。
- 首轮断言 `--session-id`。
- 后续断言 `--resume`。
- `--session-id` fixture 先返回 `Error: Session ID <uuid> is already in use.`，断言同轮只 fallback 一次、prompt 不重复落库、匹配 init 后 status=ready。
- fallback resume 返回 `No conversation found` 时断言 protocol error 且不出现第三次 spawn。
- 断言没有 `--continue` 和 `--model`。
- 断言 cwd 是 Vault root。
- 大 prompt 不进入 argv。
- stderr + non-zero exit。
- child/grandchild 被 Stop 一并终止。
- resume error 触发 unresumable。

测试 fixture 只用于测试，不成为生产 fallback。

### 17.3 前端测试

- Claude draft 不被通用 Settings autosave 提交；完整 form 误传时 Rust 白名单仍阻止三个 Claude 字段落库。
- Auto-detect 只改 draft。
- Test connection 不持久化，显示 Not saved。
- Save and connect 未预先测试也会调用连接 command。
- enabled=false 显示 Save 且不测试。
- 失败时 enabled 保留、状态 unavailable。
- current session 已绑定时显示“应用于新对话”。
- unbound/Claude unavailable 禁用发送。
- Nest-bound 不受 Claude 开关影响。
- Claude-bound disabled/unavailable/unresumable 只读。
- Claude session Ask/Agent 均能发送且无语义差异。
- Claude stream 不渲染 Sources/proposals。

### 17.4 回归检查

按仓库规范运行：

```powershell
cd apps/desktop
npm run lint
npm test
npm run build

cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## 18. 手工验收场景

### A. 配置与连接

1. 初始 Claude disabled，Nest 聊天正常。
2. 打开 Claude，路径留空，点击 Save and connect。
3. 自动检测本机 npm Claude 安装，UI 展示 resolved wrapper、CLI version、effective model。
4. 修改路径但不保存，状态显示 stale；重启后仍使用旧配置。
5. 填错路径并保存，enabled 保持 true、状态 unavailable。

### B. 首次绑定与连续会话

1. Claude enabled + connected 时新建空白会话，不立即启动 Claude。
2. 第一次发送后 session backend 变为 Claude，CLI 使用同一 Nest UUID 作为 `--session-id`。
3. 第二次发送使用 `--resume` 同一 UUID，Claude 能引用上一轮内容。
4. 新会话仍为 blank/unbound，第一条消息才创建关联。

### C. 开关与不可变绑定

1. Claude session 中关闭 Claude，不新建会话；当前 session 变只读。
2. UI 提示配置只影响新对话，并提供 New chat。
3. 新建会话后首次发送走 Nest。
4. 重新启用 Claude 后，原 Claude session 可继续，新 Nest session 仍保持 Nest。

### D. 停止与恢复

1. Claude 流式输出时点击 Stop，确认整个进程树退出。
2. 不保存半截 assistant 消息。
3. init 后停止的下一轮直接 resume 同一 ID。
4. init 前停止且 CLI 已落盘时，下一轮先收到 `Session ID ... is already in use`，随后同轮透明 resume 成功，不显示中间错误或重复用户消息。
5. 若正常 resume 明确报告 session 不可恢复，会话标为 unresumable、只读并提示新建对话。

### E. 范围边界

1. Claude Ask/Agent 两种入口得到相同行为。
2. Claude 回复无 Nest citations、Sources 或 proposal UI。
3. Claude 对话保持 `New chat`，手动重命名可用。
4. Custom models 保存后不改变实际调用模型。
5. Nest-bound 会话的 RAG、工具、引用和标题全部回归通过。

## 19. 完成定义

Step 1 只有在以下条件全部满足时完成：

- Settings 可以在 Windows 上检测并连接原生或 npm 安装的 Claude CLI。
- 未主动 Test 的配置在 Save and connect 时也能得到有效模型。
- 当前配置的连接状态可持久化并在重启后以 last-connected 语义恢复。
- 新会话在第一条消息时原子绑定后端，不在保存 Settings 时创建会话。
- Claude 首轮/后续轮分别使用 `--session-id`/`--resume` 同一 Nest UUID。
- init 前失败但 Claude 已占用 UUID 时，能够通过一次透明 resume 对账自愈，不形成重试死锁。
- 现有聊天区能展示 Claude 文本和思考流并支持 Stop。
- Claude 启用但不可用时不会静默回退 Nest。
- 已绑定会话不因全局开关改变后端。
- Claude resume 不可恢复时会话进入明确只读状态。
- Claude path 不走 Nest RAG/工具/标题；Nest path 无功能回归。
- 文档第 17 节自动测试和第 18 节手工场景通过。

## 20. Step 2 留白

以下能力必须在新的 Step 2 设计文档中定义，不从本文隐式推导：

- 聊天框内所有已启用 Agent 的一级选择。
- 每个 Agent 下的模型二级选择和 Custom models 消费方式。
- Claude 与 Nest 知识库、RAG、引用和文件操作能力的集成。
- Nest tools 对 Claude 的封装协议。
- 风险工具、权限、审批、staging、protected paths 和索引一致性。
- Claude 原生工具与 Nest 工具并存或替换策略。
- 更优的长连接/SDK/进程复用方案。
- 取消后的 transcript 完整性和更强恢复策略。

Step 1 的实现应建立稳定的 Chat Backend seam 和不可变 Session Binding，但不预先实现 Step 2 的策略 UI 或工具治理。
