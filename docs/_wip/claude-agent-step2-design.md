# Claude Agent Step 2 详细设计：Backend/Model 选择与 Nest 能力接入

> 状态：可进入开发
>
> 平台：仅 Windows
>
> 文档范围：仅 Step 2；LLM Wiki、Project Skill 与 Nest 全局控制能力后续另行设计
>
> 本文是 Claude Agent Step 2 的权威实现与验收依据；访谈记录和早期设置页草案不再作为编码输入
>
> 前置实现：[Claude Agent Step 1 详细设计](./claude-agent-step1-design.md)
>
> 术语基线：[桌面端领域上下文](../../apps/desktop/CONTEXT.md)

## 1. 目标

在 Step 1 已打通 Claude CLI 连续会话的基础上，让用户在现有聊天框中选择 Chat Backend、Model 与 Chat Mode，并让 Claude Agent 通过 Nest MCP 使用 Nest 已有的知识检索、引用、staging、Proposal 和审批能力。同时保留 Claude CLI 的成熟原生工具面，为后续 Skills、Pack 同步/发布和 Hub 控制留下扩展接缝。

Step 2 必须打通以下完整闭环：

1. composer 以 Agent → Model → Mode 三个 capsule 展示当前选择。
2. 未绑定会话在首次发送时原子绑定 Nest 或 Claude；绑定后不能混用 Backend。
3. Claude 每轮按当前 Model/Mode 启动 CLI，并通过保留名 `nest` 的 loopback MCP 获得本轮能力。
4. Ask 只获得只读工具面；Agent 保留 Claude 原生工具，并增加 Nest staged write capabilities。
5. Nest MCP 的 search/read 形成可验证 Sources，create/replace/delete 形成可审阅 Proposal。
6. Claude 原生 Bash/Edit/Write 形成 Direct Workspace Change，不伪装成 Proposal。
7. turn 完整结束后执行 Vault Reconciliation，处理索引、stage/pending proposal rebase 与冲突。
8. Tool Activities、ChatTurn、模型快照、错误与恢复状态可以持久化并在重启后正确呈现。

## 2. 已确认的产品约束

### 2.1 Step 2 包含

- Backend/Model/Mode 三 capsule 选择与 Backend Descriptor registry。
- Nest 与 Claude 两个生产 Backend；通用 adapter seam，但不实现动态插件系统。
- Claude `CLI Default` 与手工 Custom Model 的逐轮选择。
- Claude Ask 的只读原生工具和 Nest search/list/read。
- Claude Agent 的 provider-default 原生工具、用户/项目 MCP 和六个 Nest Knowledge Capabilities。
- Sources、Tool Activity、staged Proposal、Approve/Reject、确定性三方 rebase 与 conflict UI。
- Vault 生命周期 loopback MCP、session credential、active-turn lease 与 app-wide operation slot。
- 每个 Claude turn 结束后的 Vault Reconciliation、增量索引和 `reindex_required` 恢复流程。
- Save and connect 的两轮六工具真实 probe。
- Auto-detect 成功/失败的内联反馈、Custom models 行编辑器，以及 `effective_model` 向 Model options 的持久化回流。
- Step 1 数据的最低成本迁移、Claude 本地确定性标题和 Windows live acceptance。

### 2.2 继承且不得回归

- Nest Agent 与 Claude Agent 都是 Chat Backend；Ask/Agent 是 Chat Mode。
- Nest chat session UUID 与 Claude session ID 一一对应。
- 首次发送时原子 Backend Binding；全局 `agent_backend` 不是权威模型。
- Claude 模型不自动枚举，可选项仅来自 `CLI Default`、成功操作回流的 observed `effective_model` 与 `claude_custom_models`。
- Step 2 每个 Claude turn 显式传 `--model default` 或 `--model <id>`，覆盖 Step 1 省略默认模型参数的行为。
- Nest Agent 既有 RAG、引用、Proposal、审批、权限和标题行为不得回归。
- Claude 只有通过显式 Nest 集成获得的能力才叫 Knowledge Capability；原生文件工具不等同于 Nest 能力。

### 2.3 明确接受的边界与风险

- Claude Agent 是 Open Claude Runtime。Agent mode 使用 bypass/YOLO 并保留 Bash/Edit/Write，可能直接修改 Vault。
- Nest-first 是可测试的软路由，不是安全拦截；只有 Nest-mediated knowledge changes 承诺 staging、Proposal 和审批。
- Ask 是模型可见工具层的只读策略，不是 Windows sandbox；用户 hooks/plugins 属于用户信任环境。
- Step 2 不增加常驻 filesystem watcher，明确接受非 turn 期间外部修改延迟发现的窗口。
- active Pack 之外的原生文件变化不进入 Nest index、Sources 或 Proposal。

## 3. 当前实现与扩展接缝

Step 1 已提供 `chat_runtime.rs` 后端分发、`claude_cli.rs` Windows CLI 启动/解析、Claude Settings commands，以及 `ChatSession.backend/backend_status`。Claude 每 turn 启动一个进程，首轮使用 `--session-id`、后续使用 `--resume`，以 Vault 根目录为 `cwd` 并继承用户 Claude 环境。

Nest Agent 的知识工具目前集中在 `agent_tools.rs`，已具备 active Pack、protected paths、symlink、Hub 身份权限、publish-review lock、大小/数量限制、turn-local staging 和 reviewable file changes。Step 2 必须把这些协议无关规则抽成 Knowledge Workspace deep module，Rig 与 MCP adapter 共同调用，不能复制两套路径与权限实现。

```text
Composer ─► chat_send ─► Chat Runtime ─┬─► Nest Adapter ─┐
                                      └─► Claude CLI ───┼─► Capability Catalog
                                              │         │
                                              └─ MCP ───┘
                                                         │
                                      Knowledge Workspace / Review / Reconciliation
                                                         │
                                           Vault + index + proposals + activities
```

## 4. 领域模型与行为合同

本节的 D 编号是稳定需求 ID，测试名、实现说明和 review comment 应引用这些 ID。阅读顺序：D1–D11 定义聊天选择与运行时；D12–D19 定义 Knowledge/MCP 责任边界；D20–D32 定义逐轮 UI、标题与连接 probe；D33–D50 定义模块、schema、review、transport 与 Stop；D51–D53 定义持久化、Reconciliation 和生命周期恢复；D54–D56 定义 Claude Settings 的模型配置优化。

### D1. Step 2 完成边界

Step 2 同时交付两级选择、知识检索与引用、可审阅知识变更。只完成选择器或只读能力均不算完成。

### D2. Backend Selection 与 Binding

Backend Selection 只作用于未绑定的空白会话。首次发送时，选择结果与用户消息在同一事务中形成 Backend Binding；绑定后不可切换。用户在已绑定会话选择另一 Backend 时，Nest 创建新会话，不在原会话中混合两套历史或 Claude transcript。

### D3. 统一 Chat Mode 契约

- Ask：Nest catalog 只暴露 `knowledge.search/list/read`；Claude 原生工具只允许 Read、Grep、Glob、LS、WebSearch、WebFetch，不暴露 Bash、Edit、Write、Task/subagent 或其他具有工作区副作用/任意执行能力的工具。若目标 CLI 版本没有独立 LS 工具，则忽略该名称，不用 Bash 模拟。
- Agent：包含 Ask 的全部 Nest Knowledge Capabilities，并增加 staged create/replace/delete；Claude Agent 不传永久 built-in allowlist，保留该 CLI 版本的 provider-default 原生工具面。
- Claude 与 Nest 对 Knowledge Capability 使用相同 Chat Mode 契约；Backend 自身的原生工具面由 Backend Descriptor 单独声明，不能把 Claude 原生工具伪装成 Nest Knowledge Capability。

Ask 的只读承诺止于模型可见工具层，不是 Windows 文件系统沙箱：用户自行安装的 hooks/plugins 和其他 Claude 环境自定义项视为用户信任的运行环境，不属于 Nest Ask 隔离保证。UI 使用“只读工具模式”等准确文案，不声称整个 Claude 进程或其扩展绝对无副作用。

### D4. Open Claude Runtime 与分责边界

Claude Agent 在 Agent mode 下保留 Claude CLI 默认提供的原生工具能力，包括 Read、Grep、Glob、Edit、Write、Bash、WebSearch 和 WebFetch；Step 2 不使用 `--bare`、`--safe-mode` 或永久 built-in tool allowlist 把 Claude 降级为只能调用 Nest MCP 的执行器。Claude 继续以 Vault 根目录为 `cwd`，并继承当前 Claude CLI 环境中可用的配置与扩展。

由于 Step 2 继续使用非交互 `claude -p`，Agent turn 使用目标 CLI 版本文档化且经 probe 验证的 bypass/YOLO permission mode，使 provider-default Bash/Edit/Write 不因无法呈现终端确认而随机失败。Settings 和首次进入 Claude Agent mode 必须说明该模式可直接执行命令并修改 Vault；不增加每 turn 或每 tool 的 Nest 确认。该 bypass 只作用于 Claude 原生权限系统，绝不绕过 Nest Domain Capability 自身的身份、权限、锁、审阅或业务状态机。

Nest Knowledge Capabilities 是增加给 Claude 的领域能力，不是 Claude 原生工具的替代品。两类能力采用不同责任边界：

1. 通过 Nest `knowledge.create/replace/delete` 产生的写入进入 staging 和 Knowledge Change Proposal，完整复用 Nest 权限、锁、限制、冲突检查与审阅流程。
2. Claude 通过 Bash、Edit、Write 或其他原生工具直接修改 Vault 时，属于 Direct Workspace Change，不进入 Nest staging，也不能显示成已受 Nest 审阅的 proposal。
3. Nest 的 turn-end Vault reconciliation 应把直接文件变化当作外部工作区变化处理；可以刷新读取与索引状态，但不得追认或伪造 proposal。
4. 未来 Pack 同步、Pack 提交/发布、Hub 连接等具有 Nest 业务语义和内部状态机的操作，通过新的 Nest Domain Capabilities 暴露，并复用相应权限确认、业务校验和错误模型。不得要求 Claude 用 Bash 操纵 Nest 内部数据库或绕过这些领域入口。
5. Project Skills、subagents、hooks 和其他 Claude 扩展能力不因 Step 2 的安全设计被永久排除；后续阶段可以在 Backend Descriptor 和用户信任设置中声明或配置它们。

因此 Step 2 的产品承诺是“所有 Nest-mediated knowledge changes 可审阅”，而不是“Claude 产生的所有 Vault 文件变化均可审阅”。UI 与文档必须明确区分 proposal 与 Direct Workspace Change。

Claude 原生工具可以直接访问 Vault 中 active Pack 之外的路径，但这些变化不因此进入 Nest 知识边界：不产生 Citation/Proposal，不加入 active-pack index，也不参加 Knowledge effective view。Tool Activity 仍按实际可见事件显示直接操作；Nest 不把“位于 Vault 下”误判为“属于 active Knowledge Pack”。

Claude 同时遵循 Nest-first routing preference：

1. 对 active Knowledge Pack 中的 Markdown 检索与读取，优先使用 Nest `knowledge.search/read/list`，以获得 active-pack scope、effective view、可验证 citation 和 Nest 活动记录。
2. 对可由 `knowledge.create/replace/delete` 表达的 Markdown 变更，默认使用 Nest 工具生成可审阅 proposal。
3. 只有用户明确要求直接修改，或当前 Nest capability 无法表达任务时，才使用原生 Read/Edit/Write/Bash 作为 fallback；不为该 fallback 增加 Nest 专属确认弹窗。
4. 未来同步、提交/发布、Hub 连接等 Nest 业务操作不存在 Bash fallback，必须调用对应 Nest Domain Capability；原生工具不能绕过 Nest 内部授权和状态机。
5. 该规则是可测试的 Agent routing contract，不是安全隔离。运行时不承诺 Claude 永不偏离，也不能因为 prompt 中写了“优先”就把 Direct Workspace Change 标记成受管变更。

### D5. 第三方 Backend 扩展边界

Step 2 建立通用 backend descriptor、能力声明和 adapter contract，但生产实现只有 Nest 与 Claude。动态安装、发现和加载第三方 Agent/plugin 不属于 Step 2。

### D6. 空白会话的 Selection Draft

Backend/Model Selection 作为未绑定 session 的 provisional state 持久化。切换 tab 或重启 Nest 不丢失选择；首次发送时，Backend Selection 转为不可变 Backend Binding，Model Selection 按对应 Backend 的规则生效。

### D7. 新会话默认选择

新会话从创建时间最近的现有 chat session 推导默认 Backend/Model，不增加独立 preference table 或 AppSettings 字段。来源 session 已绑定时读取 immutable `backend_id`，尚未绑定时读取 `selected_backend_id`；Model 读取该 session 当前 `selected_model`。因此用户配置并选择 Claude 创建过一个会话后，下一个新会话自然继续默认 Claude，不要求先成功完成一次 turn。

继承前仍通过当前 Backend Descriptor 校验：Backend 必须 enabled/available；Model Selection 仍合法时原样继承，否则使用该 Backend 默认 Model。最近 session 的 Backend 不可用或没有历史 session 时，新会话初始化为 Nest 当前配置模型。查询必须在插入待创建 session 之前完成，并以 `(created_at, id)` 提供确定性排序。

来源 session 的 turn 成功、失败、取消、是否已有消息均不影响该默认继承。该行为只发生在创建未绑定 session 时，不是运行失败后的自动回退；创建后选择继续按 D6 持久化在新 session 上。

### D8. Claude Model 可逐轮切换

Claude 保留原生的会话内 Model 切换能力。Backend Binding 不变，但每个 turn 使用当前 Model Selection；显式 custom model 通过本轮 CLI 的 `--model` 传入。模型变化不得创建新的 Claude session ID。

### D9. Model 选项来源

- Nest Agent：只显示 Settings 当前配置的 `chat_model`，composer 不枚举其他模型。
- Claude Agent：按 D56 显示 `CLI Default`、已观察 `effective_model` 和规范化后的 `claude_custom_models`。
- 不自动枚举或预验证 custom model。
- Claude composer 每轮都绑定当前 Model Selection。`CLI Default` 映射为字面值 `--model default`，任一显式 model option 映射为 `--model <id>`；两者都覆盖 resume transcript 保存的模型。最终有效模型从 init/result 读取。

### D10. Claude 混合检索

用户显式 focus 的内容进入初始上下文；普通知识检索由 Claude 自主调用 Nest search/read capabilities。引用必须来自 Nest 实际返回的检索或读取记录，不接受 Claude 自报路径作为引用事实。

### D11. Claude 进程生命周期

Step 2 继续每 turn 启动 Claude CLI，并以 `--resume` 延续 transcript。Nest MCP service 可独立长期运行；长驻 Claude 控制协议或 Agent SDK 迁移不属于 Step 2。

### D12. 复用 Nest staging 与审批

Claude 通过 Nest Knowledge Capabilities 发起的写入复用 Nest 现有两阶段语义，不引入 Claude 专属权限确认或审阅 UI：

1. MCP 写工具自动写入 turn-local staging，不逐次弹窗。
2. staging 使用与 Nest Agent 相同的 Markdown、active pack、Hub 身份权限、publish-review lock、symlink、protected paths、数量、大小和乐观并发校验。
3. 只有 Claude turn 成功结束时，staged changes 才与 assistant message 一起持久化为 Knowledge Change Proposals；失败或取消丢弃 staging。
4. 用户继续在现有 Editor review flow 中逐文件 Approve/Reject；Approve 是 Nest-mediated change 的唯一落盘入口，并在落盘前再次校验权限、锁和磁盘内容。该约束不适用于 Claude 原生 Edit/Write/Bash 形成的 Direct Workspace Change。
5. 现有 `AgentToolContext` 不能原样作为 MCP handler。应抽出协议无关的 Knowledge Workspace 与 Knowledge Change Review 核心，Rig adapter 和 MCP adapter 只负责协议映射。

现有 Apply 流程的“读取 pending → 修改文件 → 更新数据库状态”不是并发原子操作。Step 2 在让两个 Backend 共用前必须增加 proposal claim 或按 change/path 串行化，避免并发审批失败后的 rollback 撤销另一成功写入。

### D13. Vault 生命周期级 loopback MCP

Nest 主进程为当前 Vault 维护一个长期运行的 Streamable HTTP MCP server，只绑定 loopback。每个 Claude turn 通过临时 `--mcp-config` 注入 endpoint 和受限凭证。

- Ask turn 使用 `--strict-mcp-config`，MCP 侧只提供 Nest server 的 search/list/read，避免用户或项目 MCP 破坏只读工具合同。
- Agent turn 不使用 strict MCP isolation：在 Claude 正常加载的用户/项目 MCP 基础上额外合并 Nest MCP，保留用户已有 Claude 生态。
- `nest` 是保留的固定 MCP server ID，保证工具名稳定为 `mcp__nest__...`。若用户/项目配置已有同名 server，本轮动态 Nest config 必须覆盖它并报告 `user_mcp_shadowed` warning；不得随机化 Nest ID，也不得因此使整个 Backend unavailable。其他外部 MCP 保持加载。
- init 必须单独验证名为 Nest 的 server ready。外部 MCP 初始化失败记录为 warning 和 External MCP Tool Activity，不使 Nest MCP 假装失败；Nest server 失败则本轮不能声称具有 Nest capabilities。
- 外部 MCP 的工具调用 source 标记为 `external_mcp`，不得套用 Nest 权限、citation、staging 或 proposal 语义。

Claudian 可参考的部分是：每 Vault 一个长期 listener、动态端口、功能启停联动、有界请求/响应、取消请求、关闭 listener 后等待在途写操作。不得复制其自定义 JSON-RPC、无认证 endpoint、依赖 Bash/curl 或即时写入语义。

### D14. 可扩展的 Knowledge Capability Catalog

Step 2 首批 Nest Knowledge Capabilities 严格复用现有范围：Ask 提供 search/list/read，Agent 在此基础上提供 staged create/replace/delete；catalog 本身不加入 rename、move、二进制、Hub、发布、Git 或 shell。这不限制 Claude Agent mode 使用 Claude CLI 自带的原生工具。

能力不能硬编码为 Claude 专用六项枚举。共享核心应以稳定 capability ID、输入/输出 schema、允许的 Chat Mode、风险/效果类型和 handler 注册形成 catalog，使未来 LLM Wiki、Project Skill 等项目级能力可以注册，而无需修改每个 Chat Backend adapter。LLM Wiki 与 Project Skill 已按 D18 确认为 Step 2 之后的扩展。

### D15. 引用来源

Nest 汇总本轮显式 focus 以及成功 search/read 实际返回的文件，按规范化路径去重并形成 citations。引用来自 Nest 能力调用事实，不解析或信任 Claude 在回复中自报的路径。检索结果仍应用相关性阈值，不能把所有低相关候选都展示为 Sources。

Claude 原生 Read/Grep/Glob、Web 工具和 External MCP 的路径/结果只形成 Tool Activity，不生成 Nest Citation，也不进入 Sources。Step 2 不对原生路径做二次读取提升；若回答需要 Nest Sources，Nest-first 指令要求 Claude 使用 `knowledge.search/read` 获取可验证引用。

### D16. 工具活动 UI

Claude 复用 Nest 现有 Reading files、Sources、proposal diff 和 Approve/Reject UI，并把 Claude CLI stream 与 Nest MCP 调用规范化为简洁、可折叠的 Tool Activity：

- 原生 Read/Grep/Glob/LS 显示 Reading/Searching 与可安全展示的路径。
- 原生 Edit/Write 显示 `Directly editing <path>`，明确标识 Direct Workspace Change。
- 原生 Bash 显示 `Running command` 与截断、脱敏后的命令摘要；不根据命令文本猜测具体受影响文件。
- WebSearch/WebFetch 显示联网检索/读取目标的安全摘要。
- Nest MCP search/read/list/create/replace/delete 显示 Nest 标识及 Searching、Reading、Staging 状态；只有成功形成 proposal 的 Nest 写入显示后续 Diff 和 Approve/Reject。
- 每项活动可展开查看 bounded result/error summary，但默认不展示完整参数、完整命令输出、文档正文、credential 或原始 MCP JSON。

Edit/Write/Bash 活动绝不显示 Approve/Reject，也不使用 Staged/Proposal 状态。CLI 未提供可靠路径时只显示工具类别，不从 Vault reconciliation 结果反推或伪造归因。Step 2 不构建完整调试时间线，但必须保留调用顺序、进行中/成功/失败/取消状态，并用稳定内部 activity kind 隔离不同 Backend 的原始事件格式。

### D17. 错误所有权

- Claude 拒绝 custom model：保留 Claude 的结构化错误类别，向用户展示经过截断和脱敏的 Claude 提示；Backend Binding 不变，用户可换模型重试。
- MCP transport、server 注册、认证、协议和 Nest handler 内部异常：作为 Nest 内部错误显示明确提示，不能伪装成 Claude 回复。
- 单个 capability 的参数、路径、权限或业务校验错误：作为结构化 tool result 返回 Claude，允许其在本轮修正调用；越权、认证或协议错误终止本轮。
- 无论最终工具共存策略如何，Nest MCP 不可用时不得把本轮伪装成具有 Nest Knowledge Capabilities，也不得自动回退 Nest Agent。

### D18. LLM Wiki 与 Project Skill 后移

LLM Wiki 参考 [claude-obsidian](https://github.com/AgriciDaniel/claude-obsidian)：未来可把 Vault 文档纳入可检索、带来源依据的本地知识系统，让 Chat Backend 在问答时使用。Project Skill 指维护在 Vault 或 Nest 项目根目录、激活后提供给 Claude Agent 的 Skill。

两者都是 Step 2 之后的关键特性。Step 2 不实现 Wiki ingestion、provenance ledger、Wiki UI、Skill discovery、Skill activation、Skill packaging 或 Skill execution。Step 2 只要求 backend/capability seam 不阻塞未来接入。

### D19. Session credential 与 active-turn lease

每个 Claude session 在当前 Nest runtime 内使用一个稳定、随机且不可预测的 MCP bearer credential，不要求每轮轮换。credential 只保存在进程内的 runtime credential store，以 session ID 为 key；不写入 SQLite、Settings、日志或 Windows Credential Manager。Nest 重启后旧值随进程失效，该 session 首次启动新 turn 时懒生成新 credential；Claude session ID/transcript 连续性不受影响。

MCP server 同时维护 active-turn lease：只有该 session 正在执行一个 Nest 发起的 turn 时 credential 才可调用；lease 绑定当前 turn、Vault、Chat Mode 和 capability 集合，成功、失败或取消后立即关闭。

稳定 credential 负责 session 身份，turn lease 负责时效和权限。空闲 session、已退出的旧 CLI 进程和不匹配当前 mode 的调用均被拒绝。

### D20. Chat Mode 可逐轮切换

Backend Binding 固定，但 Ask/Agent 与 Claude Model 都可在 bound session 的后续 turn 前切换。active-turn lease 根据本轮 Chat Mode 暴露 capability：Ask 只有 search/list/read，Agent 额外获得 staged create/replace/delete。切换到 Ask 不删除先前已经持久化的 pending proposals。

### D21. Turn 模型审计

每条 assistant message 在历史 DTO 中呈现 `requested_model` 与 Claude init/result 返回的 `effective_model`。其持久化权威是拥有该消息的 `ChatTurn`：前者记录发送事务冻结的 composer Model Selection，后者记录 CLI/账户/网关实际采用的模型；查询消息时通过 turn 关联投影，不能在 `chat_messages` 复制第二份可漂移状态。模型切换不会改写历史 turn 元数据。

### D21A. Claude 本地确定性标题

Claude Backend 不调用模型或隐藏 turn 生成标题。首次 `chat_send` 绑定 Claude 时，如果 session 仍为 placeholder title，则在插入首条 user message 的同一事务内，从用户可见的原始消息文本生成本地标题并把 `title_source` 设为 `local`。不使用注入的 focus 内容、Nest system instructions、Tool Activity 或 assistant reply。

标题算法必须是共享纯函数：trim，连续空白折叠为单个空格，取首个非空文本；最多保留 48 个 Unicode grapheme clusters，截断时追加 `…`。消息为空或只有不可见字符时保留 `New chat`。用户手工标题的 `manual` source 永远优先，迁移后的旧 session 不批量重命名。前端不乐观复制另一套算法，使用发送事务返回/事件中的 session title。

### D22. Bound session 切换 Backend 的 UX

Backend selector 在 bound session 中仍可操作。用户选择另一 Backend 时立即创建新的空白 session，将尚未发送的 composer 文本、focus、Chat Mode 和目标 Model Selection 移交给新 session；原 session、binding、历史和 pending proposals 保持不变。

### D23. Focus 注入

Nest 把显式 focus 文件的 bounded content、规范化路径和引用元数据注入本轮 Claude 上下文，严格复用现有上限：最多 16 个 focus 文件、每文件最多注入 6,000 字符、总计最多注入 48,000 字符。超过注入上限的内容只提供截断说明和路径，Claude 可通过 `knowledge.read` 获取允许范围内的后续内容。禁止无上限拼接完整文件或目录；现有 focus 读取失败、超时和跳过行为不为 Claude 创建旁路。

### D24. 原样复用现有能力限制

Claude 与 Nest 共用以下约束和校验：仅 active Knowledge Pack 内 Markdown；单文件 256 KiB；单 turn 最多修改 32 个文件；staged 总量最多 2 MiB；list 最多 500 项；protected paths、symlink、Hub 身份权限、publish-review lock 和内容并发校验。Step 2 不提供 Claude 专属高上限或可配置旁路。

### D25. Nest tool-chain connection test

Step 2 的 Save and connect 必须验证 CLI、有效模型、Nest MCP 注册、完整 capability catalog，以及 Claude 通过 Nest MCP 在 Vault 中创建、列出、检索、读取、替换和删除临时 Markdown 的完整路径。只验证 MCP 握手、虚拟 health tool 或只读调用不算连接成功。

Probe 运行在开放式 Agent runtime，但 challenge 明确要求对测试目标使用 Nest tools。Nest 必须根据 MCP 调用记录验证六项 capability 均被实际调用；Claude 若改用原生 Read/Edit/Write/Bash 完成测试目标，即使最终文件内容正确，连接测试仍失败并报告 `nest_tool_route_bypassed`。该判据验证 Nest-first 路径可用，不试图证明普通对话永不使用原生工具。

测试使用独立、不持久化的 Claude probe session，并允许产生和自动应用仅限测试目标的临时变更。Claude 必须通过 Nest MCP 写能力创建包含一次性 challenge 的文档，再通过 Nest MCP 读取并返回确定性结果。测试不得修改已有用户文档；测试工作区、自动 Apply、索引等待和失败清理按 D30–D32 执行。

2026-08 修订：Test connection 直接执行 D31 六工具 probe，不再额外运行独立 CLI round trip（避免三次 CLI 调用）；连接报告的 `resolved_cli_path`/`cli_version` 来自 version probe 与 detection，`effective_model` 来自 probe turn 结果。Save and connect 在内存或持久化中已存在与当前 CLI path 匹配的 Connected 报告时直接复用该报告，不重复 probe；否则执行完整测试。

2026-08 修订二：连接测试进一步简化为单轮连通性 probe——一个 headless Claude turn 仅调用 `knowledge_list`（临时 pack 上的一次只读调用），验证 CLI 启动、MCP server 应答与 Nest-first 路由可用；不再执行六工具两轮、proposal apply、索引等待与 search 验证（写路径的验收由 Knowledge Change Proposal 的既有测试覆盖）。D25 的"只读调用不算连接成功"约束放宽为"必须经真实 CLI turn 驱动 MCP 调用"。同批修订：手动 CLI path 不再强制文件名——任意 `.exe`/`.cjs` 入口或可解析到 npm wrapper 的改名 shim 均接受（auto-detect 仍按固定名扫描）。

2026-08 修订三：observed-model 来源整体退役。Model options 只由两处构成：CLI Default（当前 Connection Report 的 `effective_model`）+ 用户 Custom models；`chat_turns` 历史与旧报告不再回流选项。每个 Custom model 有持久化测试状态（`claude_model_status_v1`：ok/message/tested_at），Settings 中行内 Test 即时更新；编辑未保存的行按钮变为 Save，保存后自动触发该模型的连通测试；测试失败的模型显示 ✗ 且不进入 composer Model options。Custom model 状态在保存设置时按列表 prune。

### D26. Step 1 会话最低成本迁移

保留已有 Backend Binding；旧 Claude session 的 Model Selection 初始化为 `CLI Default`，MCP credential 首次使用时延迟创建；旧 Nest session 使用当前 Settings 模型。旧会话均为开发态测试数据，Step 2 不增加复杂 transcript 对账、批量修复或兼容 UI。

迁移不为既有 user/assistant messages 回填或猜测 ChatTurn/Tool Activity。历史消息继续按原结构显示；只有 Step 2 migration 完成后的新 `chat_send` 创建 ChatTurn。查询和 UI 必须允许 message 没有 turn/activity，不能用虚构的 succeeded turn 填补旧历史，也不清空旧会话。

### D27. Composer capsule selectors

Composer 内依次显示三个胶囊：Agent、Model、Mode。点击任一胶囊打开对应选择框；Agent 对应内部 Chat Backend，Model 随 Agent 联动，Mode 提供 Ask/Agent。胶囊始终显示当前值，发送期间禁用修改。

全局 disabled 的 Backend 不出现；enabled 但 unavailable 的 Backend 以禁用项保留，展示不可用原因和 Settings 入口。选择其他 Backend 时按 D22 创建新 session 并携带未发送草稿。

2026-08 修订：Agent 胶囊在 Backend Binding 后直接锁定（disabled + tooltip），不再提供"切换即新建 session"路径；需要换 Backend 时用户显式 New chat。Backend 下拉只显示可用后端（enabled 且 ready/last_verified）；当前绑定的后端即使变为 unavailable 也保留显示为禁用项。未绑定 session 继承的 Backend Selection 若已不可用（disabled/断连），自动回退为 Nest，保证全新 chat 总是可用。Composer 被 gate 阻塞时输入框一并禁用，而不仅是提示。

### D28. Message model label

每条 assistant message 显示轻量 effective-model 标签。requested 与 effective 不一致时，tooltip 同时展示两者；历史标签来自消息元数据，不随 session 当前选择变化。

### D29. Connection test disclosure

Settings 明确说明 Save and connect 会让 Claude 通过 Nest tools 在 Vault 内创建并处理临时 Markdown，测试完成后显示测试目标和清理结果。不增加额外确认弹窗；显式按钮行为本身即为授权。

### D30. Probe workspace 与权限复用

Nest 在当前 Vault 注册一个唯一命名的临时 local Knowledge Pack，作为本次 probe 的唯一目标。它经过与普通 local pack 相同的 active-pack、路径、Markdown、大小、锁、staging 和 review 校验。

Probe 不获得跳过权限校验的超级权限。Save and connect 是用户对该临时测试操作的显式批准；测试 proposals 仍调用同一个 Knowledge Change Review 核心自动 Apply，但调用被 test operation ID 和临时 pack root 约束，任何其他路径立即拒绝。

### D31. 两轮全工具 probe

同一个无持久化 Claude probe session 执行两个 turn：

1. Agent turn 1：create → list → read → replace → 返回 challenge → 通过正常 review 核心自动 Apply → 等待索引可见。
2. Agent turn 2：search → read → delete → 返回 challenge → 自动 Apply → 验证文件与索引均消失。

Nest 验证一次性 token、规范化 source path、预期文本片段、调用序列和每步状态；自由摘要只要求非空，不作为确定性成功判据。

### D32. Probe 清理与进度

Nest 在 `finally` 路径独立清理精确匹配 test operation ID 的文件、proposals、临时 pack、索引记录、credential 和 probe session，不依赖 Claude 的 delete。清理不完整时连接测试失败并展示残留路径。

Settings 显示创建工作区、首轮工具链、Apply/索引、续轮工具链、删除验证和清理等阶段，支持 Cancel。整个测试不设短总超时；CLI、MCP、索引和清理阶段各自使用有界超时，取消后仍必须执行清理。

### D33. Knowledge Workspace 深 Module

协议无关的外部 interface 采用 turn scope：

```text
CapabilityCatalog::for_turn(context)
KnowledgeWorkspace::open_turn(context) -> KnowledgeTurn
KnowledgeTurn::invoke(capability_id, json_input) -> CapabilityResult
KnowledgeTurn::finish() -> citations + proposals + activities
KnowledgeTurn::abort()
```

`KnowledgeTurn` 内部拥有路径解析、effective view、权限、限制、staging、引用、proposal 和错误分类。Rig 与 MCP 是同一 seam 的两个 Adapter，只转换各自 schema、事件和错误表示；连接 probe 也通过该 interface 验证行为。禁止复制权限或 staging 实现。

### D34. 可注册 BackendId

以稳定字符串 newtype `BackendId` 取代跨层封闭 enum。Step 2 注册 `nest` 与 `claude`；registry 是可执行 Backend 的权威来源。数据库中的未知 ID 保留历史并返回 unavailable descriptor，绝不执行，也不能导致 session 列表反序列化失败。

### D35. Selection 与 Binding 分离

`ChatSession` 至少区分：

```text
backend_id: Option<BackendId>       // immutable binding after first send
selected_backend_id: BackendId      // provisional selection while unbound
selected_model: ModelSelection      // mutable before each Claude turn
```

新 session 从 last-used preference 初始化 selection。首次发送在一个事务中验证 selection、写入 `backend_id` 并插入用户消息。bound session 选择另一 Backend 时创建新 session，而不是修改 `backend_id`。

### D36. Backend registry 与 descriptor

每个注册 Backend 提供统一 Backend Descriptor：stable ID、display name、enabled/availability/unavailable reason、model options、selected-model validation、supported Chat Modes、Knowledge capability profile 和 native tool profile。native tool profile 至少声明每个 Chat Mode 是 `none`、`read_only` 还是 `provider_default`；它用于 UI 告知、发送前校验和测试，不把具体 Claude 工具名泄漏为跨 Backend 的通用领域模型。执行由对应 `ChatBackendAdapter::run_turn` 完成。前端胶囊和 composer 门禁只消费 descriptors，不写死 Claude 分支。

### D37. 模块 seam 与所有权

```text
knowledge/
  workspace          权限、effective view、staging、citations、proposals
  catalog            capability descriptors 与 schema
  review             Apply/Reject、claim、rollback、索引
  rig_adapter        Nest Agent Tool adapter
  mcp_adapter        MCP tool adapter

chat_backends/
  registry           descriptors 与 adapter lookup
  nest               Nest Agent adapter
  claude             Claude Agent adapter

claude/
  cli                 CLI resolve/spawn/stream/session
  mcp_server          loopback Streamable HTTP transport
  connection_probe    两轮全工具连接测试
```

文件名可按 Rust module convention 调整，但所有权不变：Claude Module 不拥有 Nest 知识权限；Knowledge Module 不知道 Claude CLI；chat command 只编排 selection/binding、turn 和持久化。

### D38. Selection revision 与原子发送

Session 持久化 `selection_revision`。任一 Agent/Model/Mode 胶囊变更都更新 session selection 并递增 revision。`chat_send` 携带前端所见 revision，在同一数据库事务中：

1. 验证 session 和 revision。
2. 验证 descriptor availability 与 selection。
3. 若未绑定则写入不可变 `backend_id`。
4. 固化本轮 mode/requested model snapshot。
5. 插入用户消息。

revision 过期时在任何消息或 binding 写入前返回 `chat_selection_stale`，前端刷新 session 并要求用户重新发送。禁用 UI 只是体验优化，不能替代后端校验。

### D39. Capability identity 与 schema compatibility

内部 capability ID 使用稳定命名：`knowledge.search/list/read/create/replace/delete`。每个 Capability Descriptor 携带 `schema_version`、JSON input/output schema、允许的 Chat Modes 和效果类型。MCP Adapter 映射为合法工具名，例如 `knowledge_search`；Rust 类型名和 MCP 名都不是持久化身份。

增加可选字段属于同一 schema version；删除字段、改变含义或收紧既有合法输入属于破坏性修改，必须发布新版本/ID。未知 capability 或 schema version fail closed。

### D40. Backend availability

Backend Descriptor 使用通用状态：

```text
enabled: bool
availability: checking | ready | last_verified | unavailable
reason_code?: string
message?: string
```

`ready` 与路径匹配的 `last_verified` 可供新 session 选择；`unavailable` 保留禁用项。`managed_mcp_blocked`、`cli_missing`、`auth_required`、`mcp_unavailable`、`unknown_backend` 等是 reason code，不膨胀通用状态集合。

Descriptor 还为每个 supported Chat Mode 提供 mode availability/reason。模式级验证失败只禁用对应 Mode capsule 选项；例如 Claude Ask 无法可靠限制模型可见工具时标记 `ask_tool_policy_unsupported`，但 Agent mode 的 CLI 默认工具与 Nest MCP 均正常时 Backend 仍为 ready。Backend 整体不可用只用于所有受支持模式都无法启动或共享前置条件失败。

Save and connect 在 Agent 完整 probe 成功但 Ask policy probe 失败时返回 `connected_with_warnings`，保存配置并允许选择 Claude Agent；报告列出可用 Agent 与 disabled Ask/reason，Settings 和 Mode selector 都提供修复提示。它不能把 warning 显示成全连接失败，也不能继续允许选择不满足只读合同的 Ask。若 Agent probe 或 Nest MCP 全工具链失败，则仍为 `unavailable`。

### D41. Knowledge Change Review claim 状态机

Proposal review 使用持久化原子 claim：

```text
pending -> applying { claim_id } -> approved
                                -> pending | failed
        -> rebasing { claim_id } -> pending(rebased)
                                 -> conflicted | resolved_external | failed
pending | conflicted -> rejected
```

Approve 和 rebase 必须先以条件更新取得互斥 claim；同一路径同时只允许一个 applying/rebasing proposal。只有 claim owner 能完成状态或 rollback，apply rollback 前还要验证当前磁盘内容仍是本 claim 写入的内容。Nest 启动时恢复遗留 applying/rebasing：apply 根据 journal/磁盘/DB 状态完成或安全回退；rebase 未写磁盘，可重新读取三方输入后安全重试或回到 pending。无法判定时向 UI 暴露 failed，不静默覆盖。

Turn-end/startup Vault reconciliation 或任一 Knowledge Workspace/review 入口发现 staged/pending change 对应磁盘内容不再等于其 `old_content` 时，先执行确定性三方归并，而不是立即 conflicted：`base=old_content`、`proposed=new_content`、`current=当前磁盘内容`。

- Markdown 三方归并 clean 时，把 proposal 重基线为 `old_content=current`、`new_content=merged`，保持 pending，并记录 `rebase_count`、`last_rebased_at`、原 old/new hashes。UI 显示 Rebased 标识，用户审批的是 current → merged；归并结果仍不写磁盘。
- current 已等于 proposed，或 clean merge 结果等于 current 时，目标已由原生/外部修改满足，进入终态 `resolved_external`；不得标记 approved，也不再显示审批按钮。
- create/create 内容不同、modify/delete、delete/modify 或文本 edit hunks 真正重叠时进入 `conflicted`。不把 conflict markers 写入磁盘或 proposal，也不自动启动隐藏 Claude turn。
- `conflicted` 不允许 Approve，只允许 Reject；UI 保留原 proposal diff、展示 current disk，并提示需要新的 Agent turn 基于最新内容重新生成。

Approve 入口发现 baseline 变化时也先 rebase，但原来的 Approve 意图不能授权变化后的 merged diff：clean rebase 后返回 `proposal_rebased_review_required` 并要求用户重新查看、再次 Approve。`conflicted` 即使磁盘后来偶然恢复，也不自动回到 pending。

Reconciliation 不得把处于 `applying` 且磁盘内容等于该 claim 预期写入值的文件误判为外部冲突。新 proposal 成功持久化到同一路径时，原有 pending/conflicted proposal 按既有 supersede 语义转为 rejected，并记录 superseded reason。应用启动和 Vault 切换后的 reconciliation 必须扫描 pending baseline，弥补应用未运行时发生的外部修改。

现有 `snapshot::analyze_three_way/build_three_way_merge` 只在文件级选择 local/approved 版本，不能完成同一 Markdown 内的 edit-hunk 自动归并。Step 2 应在 Knowledge Review 核心新增一个协议无关、纯函数式 text three-way merge seam，由 turn-local stage、pending reconciliation 和 Approve 共用；禁止 MCP/Claude adapter 各自实现 merge。输入只接受 bounded UTF-8 Markdown/absence，并对 create/delete 矩阵、换行、相邻/重叠 hunks 和确定性输出建立单元测试。

### D42. 显式 ModelSelection

跨 Rust/TypeScript 使用 tagged type，而不是空字符串或 CLI sentinel：

```text
ModelSelection = Default | Explicit { model_id }
ModelOption = { selection, label, source: default | observed | custom }
```

数据库分别保存 kind/value；`default` 保留为 Claude Adapter 的传输映射，不能作为 custom model ID。Backend Descriptor 负责验证显式 model 是否在当前可选列表中；`ModelOption.source` 只用于 Settings/composer 解释选项来源，不参与 CLI 参数或持久化 selection identity。

### D43. Knowledge capability v1 contracts

`knowledge.search` 输入 `query` 与可选 `limit`（默认 5，范围 1–20）；scope 来自 turn context 的 active packs/focus，调用方不能扩大。返回 bounded hits：citation ID、path、title、section、snippet、score。底层复用现有 vector → FTS → lexical fallback。

`knowledge.list` 接收可选路径关键词，返回排序后的 Vault-relative Markdown paths 与 `truncated`，最多 500。

`knowledge.read` 接收一个 Vault-relative Markdown path，返回规范化 path、bounded content、citation ID 和 truncated；单次上限 256 KiB，超限返回 `limit_exceeded`。Step 2 v1 不做 cursor/line pagination。

`knowledge.create/replace` 接收 path + complete content；`knowledge.delete` 接收 path。replace 必须在同一 turn 对该 effective path 成功 read；delete 不强制 read。写入只进入 staging。

### D44. Effective-view search

所有能力观察同一 effective view：turn staged > 基线仍有效的 pending proposal > disk/index。`conflicted` proposal 不参与 overlay；list/read 直接使用有效 overlay，search 在现有索引结果之上执行 bounded lexical overlay，加入 staged/pending 的创建或修改，排除删除。未 Apply 的内容不提前写入持久化 vector/FTS index。

每次 read/stage 不能只信任上一次 reconciliation，必须在返回 pending overlay 前比较当前磁盘内容与 proposal `old_content`；发现不一致时按 D41 原子 claim 并尝试 rebase。clean 时返回 rebased overlay；resolved_external/conflicted 时退出 overlay并回退到磁盘视图。这样 Direct Workspace Change 不会在 turn-end scan 尚未执行时被旧 proposal 遮蔽。

### D45. Capability business errors

稳定 code 集合为：`invalid_input`、`not_found`、`already_exists`、`permission_denied`、`protected_path`、`review_locked`、`conflict`、`limit_exceeded`、`cancelled`、`internal`。Adapter 可改变外层表示，但 code 与 sanitized message 一致。

除 `internal` 外的业务错误可以作为 tool result 返回 Claude 修正；认证、协议、capability 越权和 `internal` 终止本轮。禁止 Adapter 通过解析英文文本重新分类。

### D46. Slice 0：Open runtime 兼容性原型门

Step 2 保持 direct CLI、现有登录和每 turn process。正式实施前的 Slice 0 不再验证“只剩 Nest MCP”的严格隔离，而验证开放运行时能否稳定注入 Nest MCP 且不破坏 Claude 默认能力：

1. 使用当前登录成功启动 Claude CLI，并保留 Agent mode 下的默认原生工具。
2. 通过临时 `--mcp-config` 注入 Nest MCP，并从 init 同时验证 Nest server ready、无相关 `mcp_server_errors`；Agent 还要验证用户 MCP 能与 Nest server 合并，单个外部 MCP 失败只产生 warning。
3. `--strict-mcp-config` 只用于 Ask 的 MCP 只读边界，不能被描述为对 plugins、skills、hooks、commands、subagents、CLAUDE.md 或 built-in tools 的完整隔离；Agent 不使用该 strict 限制。
4. fixture 必须证明 Ask turn 只有 Read/Grep/Glob/LS（若 CLI 支持）/WebSearch/WebFetch 和三个只读 Nest tools，副作用或任意执行工具不可见或不可执行；同时证明 Agent turn 的 Bash/Read/Write 与六个 Nest tools 都可用。
5. 记录已验证 Claude CLI 版本、init 特征、实际 tool exposure 和失败诊断锚点；后续 CLI 版本不满足这些能力声明时将 Claude Backend 标记为 unavailable，不静默降级。

Ask 工具列表验证失败时只把 Claude Descriptor 的 Ask mode 标记为 unavailable，并显示模式级 reason；不得改成提示词 best-effort。Agent mode 必须额外验证 bypass/YOLO permission mode 下 provider-default 工具能够在 headless 流程执行。Agent 的默认工具面与 Nest MCP 均验证成功时仍可使用。原型失败不触发 `--bare` 或 API-key-only 路线。Agent SDK 仍是未来实现 Claude 原生逐工具交互审批、checkpoint 或更深运行时控制时的独立演进选项，不属于 Step 2。

### D47. Vault 生命周期 MCP listener

当前 Vault ready 且 Claude enabled 时，Nest 在 `127.0.0.1:0` 启动一个 Streamable HTTP MCP listener，由操作系统分配端口。disable Claude、切换 Vault 或应用退出时停止接收请求、撤销 leases、取消在途调用并有界等待关闭。端口不持久化，每轮 CLI config 注入当前 endpoint。

### D48. Turn-scoped MCP transport session

每个 Claude CLI process 初始化一个新的 MCP transport session；`Mcp-Session-Id` 映射到对应 active-turn lease 和 `KnowledgeTurn`。Claude session credential 跨 turn 保持稳定，transport session 在本轮完成、失败或取消时关闭。

Step 2 在整个应用范围同一时间只允许一个 active ChatTurn，沿用当前发送状态、取消控制和单 CLI process ownership。用户可在运行期间切换 tab 查看历史，但其他 session 的 composer 禁止发送并显示当前运行会话入口。后端命令必须原子取得 app-wide turn slot，不能只依赖前端禁用；Stop/完成/失败和 turn-end reconciliation 结束后释放。

ChatTurn、lease、transport mapping 和 stream event 均保留显式 turn/session ID，不把通用数据模型写成全局单例，以便后续升级为不同 session 并发；但 MCP listener 在 Step 2 不承诺同时承载多个 active transport session。

### D49. Loopback HTTP 安全与容量

- 仅 IPv4 `127.0.0.1`，不监听 wildcard、LAN 或 IPv6 wildcard。
- Authorization 使用 bearer session credential，并要求 active-turn lease。
- Host 必须匹配实际 listener address；Origin 缺失可接受，存在时必须在明确 allowlist。
- 只接受 MCP 要求的 method 和 content type。
- 单请求最多 1 MiB，单响应最多 4 MiB。
- capability invocation 默认 30 秒；search/index/cleanup 可定义专属有界上限。
- 日志禁止记录 credential、完整文档、完整 tool 参数或未经截断的错误 body。

### D50. Stop 与 Nest system instructions

Stop 的顺序为：撤销 lease → 取消在途 capability → abort KnowledgeTurn 并丢弃未持久化 staging → 终止 Claude 进程树 → 回收 MCP transport session/reader/child。流程有界且幂等。

Nest 每轮根据 Backend Descriptor 与 Capability Catalog 生成最小固定指令，只说明 Mode、可用 Nest 能力、focus、引用规则、staging/proposal 语义、Nest-first routing preference，以及 Nest-mediated change 与 Direct Workspace Change 的区别。完整 JSON schema 只来自 MCP catalog，prompt 不复制 schema；prompt 不宣称能够阻止 Agent mode 使用原生工具。

Nest-first 指令必须给出明确 fallback 条件：active-pack Markdown 默认走 Nest；仅当用户明确要求直接修改或 catalog 无法表达任务时才使用原生文件工具。它还必须说明原生修改不会形成 Nest Proposal，Claude 不得声称此类修改已经过 Nest 审阅。

### D51. ChatTurn 与 Tool Activity 持久化

每次 `chat_send` 在首次发送原子事务中同时建立持久化 `ChatTurn`，不能等 assistant message 成功后才创建。ChatTurn 至少包含：stable turn ID、session/user-message 关联、可空 assistant-message 关联、Backend/Model/Mode/selection revision snapshot、`running | succeeded | failed | cancelled | interrupted` 状态、started/finished timestamps，以及可空 sanitized error code/message。

Tool Activity 以 turn 为所有者增量持久化，至少包含：stable activity ID、turn ID、严格递增 sequence、规范化 kind、source（native backend 或 Nest domain）、`running | succeeded | failed | cancelled | interrupted` 状态、bounded sanitized label/target/result/error summary 和 timestamps。CLI/MCP 原始 event ID 只用于本轮去重，不作为跨 Backend 领域身份；同一 activity 的状态更新必须幂等，`(turn_id, sequence)` 唯一。

持久化与恢复规则：

1. `chat_send` 的 user message、selection/binding snapshot 和 ChatTurn 在同一事务提交。
2. activity start/result 到达时分别以小事务 insert/update，使 CLI 崩溃前已经发生的 Bash/Edit/Nest 调用不会因缺少最终回复而消失。
3. 成功时 assistant message、citations、proposals 和 ChatTurn `succeeded` 在同一最终事务提交。
4. 正常错误和 Stop 完成清理后将 turn 置为 `failed`/`cancelled`，保留已记录 activities；未完成 activity 同步终结为相同状态。
5. 应用启动时，找不到存活 runtime 的遗留 `running` turn/activity 原子恢复为 `interrupted`。不自动继续工具调用或伪造 assistant reply。
6. 删除 chat session 级联删除 turns 和 activities；Step 2 不增加独立长期审计保留策略。

数据库不得持久化完整 MCP 参数/结果、文档正文、credential、Authorization header 或未处理的 stdout/stderr。Bash 摘要也必须经过 secret/path redaction 与长度限制；无法安全摘要时只保存工具类别和状态。前端实时事件与历史查询使用同一个规范化 Tool Activity DTO，避免重开会话后呈现不同语义。

Claude 文本继续按现有 stream 实时显示。收到 terminal result 后，前端保留已经显示的临时 assistant bubble并进入 `Finalizing workspace…`；composer 和跨 session send 继续禁用。Sources 可显示为 provisional activity，Knowledge Proposals、Rebased/conflicted/resolved_external 和最终 warning 只能在 finish/reconciliation 后提交并附加。finalizing 中应用崩溃时，未进入最终事务的临时文本不作为正式 assistant message恢复，ChatTurn 显示 interrupted 与已持久化 Tool Activities。

### D52. Turn-end Vault reconciliation

Step 2 不新增常驻 filesystem watcher。每个 Claude turn 在 CLI 进程已正常退出或被 Stop/错误路径完整终止后，都扫描 active Knowledge Packs 中的 Markdown manifest，并与上一次持久化的索引 manifest/hash 比较；成功、失败和取消 turn 都必须执行，因为 Direct Workspace Change 可能早于最终结果发生。

Reconciliation 对新增、修改和删除路径执行增量索引更新，并逐项核对本轮 staged change 与已有 pending proposal baseline；发生偏离时按 D41 先尝试确定性三方 rebase，只有不能 clean merge 才标记 conflicted。本轮 stage clean rebase 后以 current → merged 生成新的 pending Proposal；已有 pending clean rebase 后原子更新其 old/new 与 rebase metadata。扫描/归并/索引期间保持应用级 active-turn gate，完成后才允许下一次发送，保证下一 turn 的 Nest search/read 不使用已知过期索引。大量变化或 manifest 不可信时允许调用现有全量 index rebuild，但必须有阶段状态和有界超时。

Reconciliation 失败或超过有界 timeout 不撤销 Claude 已经完成的 Direct Workspace Change，也不伪造回滚；ChatTurn 保留其原执行结果并附加 `workspace_reconciliation_failed` warning，持久化全局 `reindex_required`，释放 active-turn slot并提供 Reindex 入口。若 staged paths 的独立最终 baseline/rebase 已安全完成，回复和对应 Proposal仍可提交；若该验证本身失败，则相关 staging 不得持久化为可审批 proposal。

`reindex_required` 期间 `KnowledgeWorkspace::open_turn` fail closed，Nest search/list/read/create/replace/delete 对所有 Backend unavailable，避免使用已知过期索引或 proposal baseline。Nest Agent 因依赖该 workspace 标记 unavailable；Claude Agent 的原生 tools 和 External MCP 仍可使用，Descriptor/UI 明确显示 Nest Knowledge unavailable。此时 Claude Agent turn 仍注入受认证的 reserved `nest` server 以遮蔽同名用户 MCP，但该 server 的本轮 Capability Catalog 为空，`tools/list` 不暴露六个 Knowledge tools，system instructions 明确说明 Nest Knowledge unavailable；不得保留工具名后等调用时才返回过期状态。成功 Reindex 与 pending proposal reconciliation 在同一恢复流程完成后清除此标志。Reindex 本身取得 app-wide operation slot，不能与 ChatTurn/probe 并发；Save and connect 的六工具 probe 必须先要求完成 Reindex。

该方案明确接受非实时窗口：Claude turn 结束后的后台进程或 Nest 之外的外部编辑不会立即刷新索引，也不会立即把 proposal 标记 conflicted；下一次 Claude turn reconciliation、Knowledge read/stage、Approve 或应用启动/Vault 切换 reconciliation 才会发现。Step 2 文档不得暗示存在实时 Vault watcher。

### D53. Active-turn lifecycle mutations

删除当前正在生成的 chat session 时，Nest 自动执行完整 Stop：撤销 lease、取消 capability、abort stage、终止并回收 Claude 进程树、关闭 transport，并运行 bounded reconciliation；随后删除 session 及级联 messages/turns/activities/proposals。原生工具已经落盘的 Direct Workspace Change 不回滚。Reconciliation 失败不永久阻塞用户明确的删除，改为持久化全局 `reindex_required` warning，并保留 Reindex 入口。

active ChatTurn 或 connection probe 期间禁止修改 Claude enable/CLI path、执行 Save and connect，或切换 Vault/knowledge directory。对应控件 disabled，后端命令也必须检查 app-wide operation slot。修改 Vault 路径时显示专用提示：`Claude Agent is using the current Vault through Nest MCP. Stop the current task before changing the Vault location.` 用户必须显式 Stop，不在普通 Settings 保存中自动终止 Agent。

应用退出时执行有界清理：撤销全部 leases、取消在途 capability、abort stage、终止 Claude/子进程树并尽力执行短时 reconciliation。退出不能无限等待大型扫描；未完成的 ChatTurn/Activity 在本次可写时标记 interrupted，否则下次启动统一恢复。启动恢复必须终结遗留 running 状态、重新核对 pending proposal/rebase claims 与 active-pack manifest/index。已经发生的 Direct Workspace Change 不回滚。

### D54. Auto-detect 结果反馈

`Auto-detect` 不是只弹 toast 的动作。成功时必须同时：

1. 把 `ClaudeDetectionDto.resolved_path` 填入未保存的 CLI Path draft。
2. 在路径输入框下方显示独立检测信息行，至少包含 resolved path、CLI version 和成功状态；该行不与 Save/Test 的 Connection Report 混为一体。
3. 将 draft 标记为 dirty，但不持久化、不改变 connected/last-connected 状态。

用户再次编辑 CLI Path 后，旧检测信息立即失效并隐藏。检测失败时保留用户已经输入的路径并显示内联失败信息；若路径原本为空，输入框使用明确 placeholder `Claude CLI not found — enter the path manually`，不能只显示瞬时 toast 后恢复成无法判断的空白状态。重新检测、Test 或 Save 成功后清除失败 placeholder。检测期间按钮与路径输入按同一 mutation 禁用，结果使用最后一次请求身份，过期响应不得覆盖较新的 draft。

### D55. Custom models 行编辑器

Settings 不再用多行 `Textarea` 编辑 `claude_custom_models`，改为有序行编辑器：每行一个 model ID 输入框和删除按钮，末尾提供 `Add model`；在当前行按 Enter 也创建并聚焦下一行。至少保留一个可编辑空行，空行不持久化。删除最后一个非空模型后仍显示空行。

UI hydrate 时把现有 newline 字符串解析为 rows；Save 时逐行 trim、移除空行、按精确字符串去重并保持首次出现顺序，再序列化回既有 newline 字段，避免 Step 2 为纯 UI 优化引入破坏性 Settings migration。重复行显示内联提示且 Save 结果仍以规范化规则为准。行编辑器必须支持键盘操作、可访问 label，并在 active operation 期间整体禁用；Step 2 不要求拖拽排序、模型存在性预验证或自动枚举账户模型。

### D56. `effective_model` 回流 Model options

Claude Backend Descriptor 的 Model options 由三类来源按顺序合并并按精确 ID 去重：

1. 固定的 `CLI Default`（`ModelSelection::Default`）。
2. 非空的已观察 `effective_model`，最新观察优先。
3. 用户 Custom models，保持行编辑器顺序。

“已观察”来源仅包括成功 Test/Save 的当前 `ClaudeConnectionReport.effective_model`，以及成功 Claude ChatTurn 持久化的 `chat_turns.effective_model`；失败、取消、interrupted、空值和仅由 Claude 文本自报的模型不得加入。历史查询最多取最近 20 个不同 observed model ID，避免选项无限增长。Connection Report 和 ChatTurn 已是持久化事实源，不新增另一份 observed-model Settings，也不自动改写用户的 Custom models。

每个 option 带 D42 的 `source`。同一 ID 同时为 observed/custom 时只显示一项并标为 `observed`，Custom models 行编辑器仍保留用户原始行；删除 Custom row 不会删除仍有观察事实支持的 option。

Test connection 的未保存结果应立即回流 Settings 中的只读 `Detected models` 列表；由于 Test 不持久化配置，若测试的 draft 与已保存 Claude 配置不一致，不得把该结果泄漏到 composer。这里的配置指纹沿用 Step 1 `matches_configured` 语义，即规范化后的 configured CLI path，且 Backend 本身必须已启用。Save and connect 持久化 Connection Report 后，或成功 ChatTurn 提交后，invalidate Backend Descriptor/session queries，使模型下一次打开 composer 选择框即可出现。若 Test 的配置指纹与已保存配置完全一致，descriptor 可以直接使用当前 runtime report。观察到实际模型不会把当前 `CLI Default` selection 自动改成 Explicit，也不会改写任一 session 的 Model Selection；用户主动选择 observed ID 后才作为 `ModelSelection::Explicit` 逐轮传入。

2026-08 修订：Settings 不再显示只读 `Detected models` 列表（旧默认模型会与新默认累积展示，且信息与 Custom models 重复）。当前 Connection Report 的 `effective_model` 改为在 Custom models 行编辑器中显示为只读首行并标记 `[default]`，随每次成功 Test/Save 自动更新为新默认模型；该行不写入 `claude_custom_models`，用户的 Custom models 行仍由用户独占，composer Model options 的合并来源不变。

## 5. Claude CLI / MCP 实施前提

1. Headless CLI 支持 `--mcp-config <file-or-inline-json>` 临时注入 MCP；`--strict-mcp-config` 可排除用户、项目和 plugin MCP，但企业 managed MCP policy 可能禁止动态 server。
2. `--tools` 管理 Claude built-in tools，不限制 MCP。Nest MCP 工具命名为 `mcp__nest__<tool>`；`--allowedTools` 只表示自动批准，不是可见性 allowlist。
3. `system/init.mcp_servers` 和 `mcp_server_errors` 必须参与启动校验，因为无效 MCP server 不一定令 CLI 非零退出。
4. `-p` 支持 `--permission-prompt-tool`，但官方没有完整稳定的 direct CLI approval schema；不能把它作为 Step 2 唯一审批基础而不先做兼容性原型。
5. resume 默认恢复 transcript 模型；新进程显式传 `--model` 可以覆盖本轮模型，但会重读历史并可能损失 prompt cache。
6. Windows 可使用 stdio MCP sidecar 或 loopback Streamable HTTP。HTTP 必须只绑定 loopback、校验 Origin 并认证；stdio server 必须保证 stdout 只承载 JSON-RPC。

### 5.1 不采用严格隔离路线

截至本文调研日期，direct CLI 没有一个被官方完整保证的参数组合可以同时满足“沿用现有 OAuth/keychain 登录”与“关闭全部本地自定义发现后只重新注入 Nest MCP”：

- `--bare` 明确关闭 hooks、skills、commands、subagents、plugins、MCP、auto memory 和 CLAUDE.md，并允许通过显式 `--mcp-config` 注入 Nest；但它不读取 OAuth/keychain/`CLAUDE_CODE_OAUTH_TOKEN`，只能使用 API key 或 `apiKeyHelper`。
- `--safe-mode` 保留认证并关闭本地自定义项，但官方只声明 MCP 不加载，未保证显式 `--mcp-config` 可重新启用。
- direct CLI 未文档化 `--setting-sources ""`；Agent SDK 才明确支持空 settings sources。
- `--strict-mcp-config` 只隔离 MCP 来源，不禁用 plugin 的 skills、agents、hooks 或 commands。
- managed policy、默认 Claude system prompt、继承环境和既有 resume transcript 仍可能影响运行。

这些限制解释了为什么 Step 2 不再把“沿用现有登录且只保留 Nest MCP”作为运行时合同。它们仍是未来增加严格安全级别时的输入，但不再构成当前实现门。

### 5.2 Claudian 安全模型对照

Claudian 不采用 Nest 式严格工具面。其主聊天使用 provider-default tools，fresh 默认 permission mode 为 YOLO/bypass；Claude cwd 是 Vault，原生 Edit/Write/Bash 可以直接修改文件。Agent SDK 的 `canUseTool` 可在需要 permission 时提供 Allow once / Always allow / Deny，但既有规则、acceptEdits 或 bypass 可以跳过询问。

Claudian 的 file checkpoint/rewind 是写入后的恢复能力，不是 turn-local staging、proposal 或写前审阅。其产品采用 Claude Code 原生信任模型，并不承诺所有 Vault 写入都经过宿主 review。

Nest 当前 direct CLI 架构没有 Agent SDK 等价的稳定 `canUseTool` callback；direct CLI 的 `--permission-prompt-tool` schema 仍缺乏完整官方合同。因此若保留原生写工具，不能直接复制 Claudian 的宿主审批体验而不重新打开 SDK 决策。

对照分析包含三类信任模型：

1. Nest-only：只暴露 Nest MCP，最强治理，牺牲全部 Claude 原生能力。
2. 分权混合：保留经审计的只读/联网原生能力；原生 Edit/Write/Bash、foreign MCP 和可执行逃逸面禁用；所有 Vault 写入仍走 Nest MCP proposal。
3. Claude-native：provider-default + Claude permissions/checkpoint；只能承诺 Nest tool 写入可 review，不能承诺所有 Vault 写入可 review，并可能要求迁移 Agent SDK。

Step 2 采用第三类的开放工具方向，但不复制 Claudian 的 SDK permission callback 或 checkpoint UI：继续 direct CLI，由 Claude CLI 管理其原生工具；Nest 只对自己的 Domain Capabilities 做权限、审批、状态与错误治理。若未来需要由 Nest 逐次批准 Bash/Edit/Write，再单独评估 Agent SDK。

## 6. 数据设计

### 6.1 `chat_sessions` 增量字段

保留 Step 1 既有 `backend` 物理列作为 immutable Backend Binding，不为了命名美观执行破坏性 rename；Rust/TypeScript 领域字段可映射为 `backend_id`。新增：

| 字段 | 类型 | 约束与含义 |
|---|---|---|
| `selected_backend_id` | TEXT NOT NULL | 未绑定时的 Backend Selection；首次绑定时设为与 `backend` 相同，绑定后不在原 session 改写；选择另一 Backend 由 D22 创建新 session |
| `selected_model_kind` | TEXT NOT NULL | `default` 或 `explicit` |
| `selected_model_value` | TEXT NULL | explicit 时非空；default 时必须 NULL |
| `selection_revision` | INTEGER NOT NULL | 初始 0，每次 Backend/Model/Mode selection 改变递增 |

`mode` 继续使用既有列，但 capsule 修改必须与上述 selection 一起走 revision-aware command。数据库 CHECK 无法覆盖旧 SQLite 时由 repository method 强校验 tagged model invariant。

迁移规则：已有 `backend=claude`/`nest` 原样保留；未绑定旧 session 的 selection 初始化 Nest。旧 Claude session model 为 Default；旧 Nest session 使用迁移时当前 `chat_model` 构造 Explicit。迁移不改变 session UUID、Claude transcript ID、消息、标题或 proposal。

### 6.2 `chat_turns`

新增表：

```text
id TEXT PRIMARY KEY
session_id TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE
user_message_id TEXT NOT NULL UNIQUE REFERENCES chat_messages(id) ON DELETE CASCADE
assistant_message_id TEXT NULL UNIQUE REFERENCES chat_messages(id) ON DELETE SET NULL
backend_id TEXT NOT NULL
requested_model_kind TEXT NOT NULL
requested_model_value TEXT NULL
effective_model TEXT NULL
mode TEXT NOT NULL
selection_revision INTEGER NOT NULL
status TEXT NOT NULL
error_code TEXT NULL
error_message TEXT NULL
warnings_json TEXT NULL
started_at TEXT NOT NULL
finished_at TEXT NULL
```

status 只允许 `running/succeeded/failed/cancelled/interrupted`。`warnings_json` 是 bounded stable warning code array，不保存原始 stderr。`chat_turns` 是 Backend/Model/Mode 执行快照的唯一持久化权威；`chat_list_messages` 把关联 turn 的模型字段投影进 assistant message DTO 供历史渲染，不向 `chat_messages` 复制这些列，也不能从 session 当前 selection 回读。

### 6.3 `chat_tool_activities`

新增表：

```text
id TEXT PRIMARY KEY
turn_id TEXT NOT NULL REFERENCES chat_turns(id) ON DELETE CASCADE
sequence INTEGER NOT NULL
source TEXT NOT NULL
kind TEXT NOT NULL
status TEXT NOT NULL
label TEXT NOT NULL
target TEXT NULL
result_summary TEXT NULL
error_code TEXT NULL
error_message TEXT NULL
started_at TEXT NOT NULL
finished_at TEXT NULL
UNIQUE(turn_id, sequence)
```

source 为 `claude_native/nest_mcp/external_mcp`；status 为 `running/succeeded/failed/cancelled/interrupted`。kind 使用稳定值如 `file_read/file_search/file_edit/bash/web_search/web_fetch/knowledge_search/knowledge_read/knowledge_stage/external_tool`，不能持久化 Claude/MCP 原始 type 作为 UI contract。所有文本在 repository boundary 前完成长度限制与脱敏。

### 6.4 Proposal 增量字段与状态

扩展 `chat_file_changes`：

| 字段 | 用途 |
|---|---|
| `claim_id`, `claim_kind`, `claimed_at` | `apply/rebase` 原子 claim 与恢复 |
| `failure_code`, `failure_message` | failed/conflicted 的稳定诊断 |
| `resolution_reason` | superseded、already satisfied externally 等终态原因 |
| `rebase_count`, `last_rebased_at` | rebase UI 与恢复 |
| `rebased_from_old_hash`, `rebased_from_new_hash` | 不保存额外全文的来源证据 |
| `apply_expected_hash`, `apply_journal_json` | Apply 写入后 DB 失败时的归属验证与恢复 |

status 扩展为 `pending/rebasing/applying/approved/rejected/conflicted/resolved_external/failed`。旧 approved/rejected/pending 数据保持。任一 repository 查询不得把非 pending 状态加入 effective view。

### 6.5 Workspace manifest 与健康状态

新增 active-pack manifest 表，至少保存 normalized path、content hash、size、mtime/identity hint 和 last indexed timestamp。mtime/size 只用于快速筛选，content hash 才是 Proposal baseline 与 index freshness 的事实。

新增 singleton workspace health（可扩展现有 `index_meta`）：`reindex_required`、stable reason、updated_at。应用启动先恢复 claims/running turns，再 reconciliation manifest；健康状态未恢复前不能开放 KnowledgeWorkspace。

### 6.6 Claude Model option 投影

不新增 observed-model 表。Repository 提供 bounded query，从当前匹配配置的已保存 `ClaudeConnectionReport.effective_model` 与 `chat_turns` 中筛选 `backend_id=claude AND status=succeeded AND effective_model IS NOT NULL/empty`，按最近观察时间倒序返回最多 20 个不同 ID。Backend registry 在该结果之上按 D56 合并 `CLI Default` 与 Custom models。

未保存 Test connection 的 `effective_model` 只存在于 Test mutation result/runtime report，作为 Settings 的 `Detected models` 临时预览；关闭 Settings 或重启后不保留。只有测试配置指纹等于已保存配置时，Backend Descriptor 才可纳入该 runtime report。Save and connect 的 Connection Report 和正常 ChatTurn 继续使用各自既有事务持久化，不为了刷新列表写入 `claude_custom_models`。

## 7. 后端模块与 Rust 接口

### 7.1 Backend registry

```rust
trait ChatBackendAdapter {
    fn descriptor(&self, ctx: &BackendContext) -> BackendDescriptor;
    async fn run_turn(&self, ctx: ChatTurnContext) -> AppResult<ChatBackendResult>;
}
```

`BackendDescriptor` 必须包含 stable ID、label、enabled、availability/reason、每 Mode availability、model options/default、native tool profile、Knowledge capability profile 和 Settings target。Claude model options 由 repository projection + D56 的纯函数构造，前端不得自己合并 Connection Report、turn history 与 Custom models。Registry 在启动时拒绝重复 ID；未知持久化 ID 保留历史但 descriptor unavailable。

### 7.2 Selection commands

新增/替换为：

```text
chat_backend_descriptors() -> BackendDescriptor[]
chat_update_selection(session_id, expected_revision, patch) -> ChatSession
chat_send(session_id, expected_revision, query, focus_paths, protected_paths, event_name) -> ChatMessage
```

`chat_update_selection` 验证 Backend/Model/Mode descriptor 后以 compare-and-update 递增 revision。bound session 的 Backend 变化由 command service 创建并返回新 session，同时迁移未发送 draft 由前端完成；后端绝不改写原 binding。`chat_send` 不再相信前端重复传来的 backend/model/mode 值，只使用事务内锁定的 session selection snapshot。

### 7.3 Review 与历史 commands

```text
chat_list_messages(session_id) -> messages + optional turn/activity summaries
chat_list_turn_activities(turn_id) -> ToolActivity[]
chat_review_file_change(change_id, action) -> ReviewOutcome
chat_cancel() -> StopOutcome
```

`ReviewOutcome` 区分 `approved/rejected/rebased_review_required/conflicted/resolved_external/failed`，前端不得通过错误字符串猜测。活动列表可按消息查询拆分，但 DTO 必须与 stream event 相同。

### 7.4 Knowledge Workspace deep module

```rust
CapabilityCatalog::for_turn(ctx) -> CapabilityCatalog
KnowledgeWorkspace::open_turn(ctx) -> KnowledgeTurn
KnowledgeTurn::invoke(capability_id, json) -> CapabilityResult
KnowledgeTurn::finish() -> KnowledgeTurnOutcome
KnowledgeTurn::abort()
KnowledgeReview::review(change_id, action) -> ReviewOutcome
KnowledgeReview::reconcile(paths_or_manifest) -> ReconcileOutcome
```

`KnowledgeTurnOutcome` 包含 citations、proposals、activities 与 warnings。所有 path normalization、active-pack scope、limits、effective view、stage、rebase 和 permission 位于该 module；Rig/MCP adapter 不得直接访问 Vault/DB 实现旁路。

## 8. Claude CLI 与 MCP 设计

### 8.1 公共参数

每轮继续使用 `-p --output-format stream-json --verbose --include-partial-messages`，stdin 仅传用户 query 与 bounded focus envelope。首轮 `--session-id <nest-session-id>`，后续 `--resume`；保留 Step 1 的 ID-in-use → 单次透明 resume 自愈。每轮显式传 `--model default` 或 `--model <explicit>`。

Nest 固定指令通过目标 CLI 已文档化的 append-system-prompt 能力注入，不替换 Claude 默认 system prompt；Slice 0 必须验证 resume 每轮仍读取本轮 Mode/capability 指令。若目标版本不支持稳定 append contract，则把同一 envelope 作为有明确 delimiter 的受信任前缀加入 stdin，并在兼容性报告中记录 transport，不能混入 focus 文档内部。

### 8.2 Ask

```text
permission mode: default/read-only-compatible
built-in tools: Read,Grep,Glob,LS(if supported),WebSearch,WebFetch
MCP config: only reserved nest server
strict MCP config: enabled
auto-approved MCP: mcp__nest__knowledge_search/list/read
```

init/tool exposure 与 probe 不满足列表时 Ask mode unavailable；不能只靠 prompt。

### 8.3 Agent

```text
permission mode: bypass/YOLO
built-in tools: provider-default (omit permanent --tools allowlist)
MCP config: user/project MCP merged with reserved nest server
strict MCP config: disabled
Nest MCP tools: all six v1 capabilities
```

CLI flag 名称随受支持版本由 adapter capability probe 选择，但领域语义不能变化。Nest 临时 MCP config 放在 app-owned temp directory，限制普通跨用户读取，turn finally 删除；任何 bearer/header 不进入 debug args log。

### 8.4 Stream parser

扩展现有 parser 处理 assistant content blocks 中的 `tool_use/tool_result`、MCP activity、init tool/server metadata。parser 只产出 normalized candidate events；redactor/Activity repository 再决定可持久化字段。未知 block 忽略并记录 bounded protocol warning，不因 Claude 新增非关键 block 崩溃；session mismatch、认证、Nest MCP 身份或 terminal result 矛盾仍 fail closed。

## 9. 对话发送与 Finalization 流程

```text
acquire app-wide operation slot
  -> transaction: validate revision/descriptor, bind, title, user message, ChatTurn(running)
  -> open KnowledgeTurn + active lease
  -> spawn Claude CLI and MCP transport
  -> stream tokens/thinking/Tool Activities
  -> receive terminal result or Stop/error
  -> revoke lease, cancel in-flight work, close/kill/reap runtime
  -> finish stage or abort
  -> Finalizing: deterministic rebase + Vault reconciliation + index update
  -> transaction: assistant/citations/proposals/turn terminal state/warnings
  -> emit final session/message state
  -> release operation slot
```

所有错误分支都必须汇合到一个幂等 finalizer。禁止在多个 command/adapter 中分别 kill child、abort stage 或释放 lease。finalizer 接受 terminal intent（success/fail/cancel/delete/shutdown）并返回可测试的 cleanup report。

## 10. 前端详细设计

### 10.1 Composer

三个 capsule 固定顺序 Agent → Model → Mode。Session query 是 selection/revision 的唯一前端事实；发送 payload 带 expected revision。bound Backend 切换创建新 session并搬运当前内存 draft/focus，不能复制历史或 proposals。

全应用 operation slot busy 时所有 composer 禁用；当前 session 展示 Stop，其他 session 展示“另一会话正在运行”并提供跳转。`reindex_required` 时 descriptor 显示 Nest Knowledge unavailable；Claude Agent 原生能力仍可选择并显示 warning。

### 10.2 Message 与活动

临时 assistant bubble承载 token stream；Tool Activity 按 stable ID upsert，不能每个状态生成重复行。默认折叠成功活动，running/failed/direct edit 适度突出。历史重载从 ChatTurn DTO 恢复相同顺序和标签。

Sources 只渲染 Nest citations。Native/External MCP read 永远留在 activity 区。Proposal 卡片支持 pending/rebased/conflicted/resolved_external/applying/failed；rebased 显示当前 diff 和 badge，Approve-triggered rebase 刷新卡片并要求第二次点击。

### 10.3 Settings

Save and connect 显示 probe phases、Cancel、临时 Pack 和清理结果。connected-with-warning 分开展示 Agent ready 与 Ask unavailable。Agent bypass/YOLO 风险在设置分组常驻，并在首次选择 Claude Agent mode 时显示一次说明；它不是逐工具确认。

CLI Path 区按 D54 分离 `DetectionResult` 与 `ConnectionReport`：Auto-detect 成功填 path 并显示 resolved path/version 信息行，失败提供持久到下一次编辑/成功操作的内联状态与空路径 failure placeholder。Custom models 区按 D55 使用行编辑器，不再渲染 Textarea，并在其上方或下方显示只读 `Detected models`。Test/Save/成功 turn 得到 `effective_model` 后按 D56 刷新对应预览或 descriptor；不得把 observed model 伪装成已保存 Custom model。

active operation 时 Claude 保存、disable、重新测试和 Vault 路径控件前后端同时门禁。`reindex_required` warning 是全局 workspace banner，Reindex 成功后自动刷新 descriptors、sessions、messages、tree 和 Sources queries。

## 11. 停止、错误与恢复

| Code | 类型 | 行为 |
|---|---|---|
| `chat_selection_stale` | error | 无写入，刷新 session 后重发 |
| `chat_turn_busy` | error | 跳转/Stop 当前 active turn |
| `backend_unavailable` | error | descriptor reason + Settings |
| `chat_mode_unavailable` | error | 禁用该 Mode，不自动换 Mode |
| `ask_tool_policy_unsupported` | warning/mode reason | Agent 可继续，Ask disabled |
| `nest_mcp_unavailable` | error | 本轮不能声称 Nest capability |
| `nest_tool_route_bypassed` | probe error | 连接测试失败 |
| `user_mcp_shadowed` | warning | 保留 Nest server ID，Agent 可继续 |
| `external_mcp_unavailable` | warning | 只影响对应外部工具 |
| `proposal_rebased_review_required` | review outcome | 刷新 diff，要求重新 Approve |
| `proposal_conflicted` | review outcome | 禁止 Approve，可 Reject/新 turn |
| `workspace_reconciliation_failed` | warning | 设置 reindex_required |
| `nest_knowledge_reindex_required` | error/availability reason | 禁用 KnowledgeWorkspace |

Claude Step 1 的 CLI/session/protocol/model codes继续使用。Capability business codes维持 D45。所有 UI 分支按 code/typed outcome，英文 message 只用于展示，不参与控制流。

## 12. 文件级实施清单

开发时允许按现有模块风格微调文件拆分，但以下所有权不能跨层复制：

| 文件 | 主要改动 |
|---|---|
| `packages/shared/src/index.ts` | 增加 Backend Descriptor、ModelSelection、ChatTurn、ToolActivity、ReviewOutcome、workspace health 与 stream DTO；扩展 ChatSession/message DTO |
| `apps/desktop/src-tauri/src/db.rs` | schema migration、selection revision CAS、ChatTurn/activity repository、proposal claim/rebase metadata、manifest/workspace health，以及 D56 的 bounded distinct effective-model query |
| `apps/desktop/src-tauri/src/state.rs` | Backend registry、Vault-lifecycle MCP listener、session credential store、active-turn lease、app-wide operation slot 与 recovery state |
| `apps/desktop/src-tauri/src/chat_runtime.rs` | `ChatBackendAdapter` 分发、descriptor availability、统一 finalizer；不得包含 Backend 专属 CLI/MCP 细节 |
| `apps/desktop/src-tauri/src/chat_history.rs` | message + ChatTurn + ToolActivity 历史投影，保持旧 Step 1 消息兼容 |
| `apps/desktop/src-tauri/src/agent_tools.rs` | 将现有 Nest 工具改为 Knowledge Workspace/Rig adapter 的薄接入层，不保留第二套权限与 staging 规则 |
| `apps/desktop/src-tauri/src/knowledge_workspace.rs`（建议新增） | Capability Catalog、turn context、effective view、bounded schema、staging、citations 与协议无关业务错误 |
| `apps/desktop/src-tauri/src/knowledge_review.rs`（建议新增） | proposal claim、Apply/Reject、确定性文本三方 rebase、conflicted/resolved recovery |
| `apps/desktop/src-tauri/src/vault_reconciliation.rs`（建议新增） | manifest scan、增量索引、proposal reconciliation、`reindex_required` 与 Reindex；不实现 watcher |
| `apps/desktop/src-tauri/src/claude_mcp.rs`（建议新增） | loopback Streamable HTTP、bearer/lease、Host/Origin/body limits、reserved `nest` server、turn transport |
| `apps/desktop/src-tauri/src/claude_cli.rs` | Ask/Agent 参数矩阵、MCP config merge、每轮 model、system instructions、tool stream parsing、process-tree cleanup |
| `apps/desktop/src-tauri/src/chat_events.rs` | normalized Tool Activity、Finalizing、workspace warning 与 terminal state events |
| `apps/desktop/src-tauri/src/agent.rs` | Nest Adapter 使用共享 catalog/workspace，保证现有 Nest 行为不回归 |
| `apps/desktop/src-tauri/src/retrieval.rs`、`indexing.rs`、`snapshot.rs` | effective-view search、增量索引接缝；新增共享文本级 merge，不能把现有 file-choice merge 当作 D41 实现 |
| `apps/desktop/src-tauri/src/commands/chat.rs` | revision-aware selection/send、原子 bind/title/user message/ChatTurn、history/review/Stop 与 bound-switch new-session 用例 |
| `apps/desktop/src-tauri/src/commands/claude.rs` | Auto-detect DTO/错误反馈、完整两轮 probe、effective model、partial mode status、Cancel/cleanup；复用生产 MCP/CLI adapter |
| `apps/desktop/src-tauri/src/commands/settings.rs`、`vault.rs`、`index.rs` | active-operation 门禁、Vault 提示、Reindex command 与状态恢复 |
| `apps/desktop/src-tauri/src/commands/mod.rs`、`lib.rs`、`Cargo.toml` | 注册新模块/commands；加入经 Slice 0 证明必要的 HTTP/MCP/merge 依赖 |
| `apps/desktop/src/lib/api.ts`、`query-keys.ts`、`claude-composer.ts` | typed commands/events、selection revision、descriptor/query keys、effective-model overlay、capsule/Custom-model rows 派生纯函数 |
| `apps/desktop/src/components/chat/ChatPanel.tsx` | app-wide busy、流式 turn、Finalizing、Stop、warning、history hydration |
| `apps/desktop/src/components/chat/MentionComposer.tsx` | Agent → Model → Mode capsules、availability、bound-switch session transfer |
| `apps/desktop/src/components/chat/ChatFileChanges.tsx`、`AgentStatusIndicator.tsx` | Proposal 新状态、re-review/conflict UI、Tool Activity 与 Direct Workspace Change 标识 |
| `apps/desktop/src/components/settings/ClaudeAgentSettingsSection.tsx`、`SettingsPanel.tsx` | D54 检测信息行/failure placeholder、D55 Custom models 行编辑器、detected model、probe phases、connected-with-warning、门禁与风险文案 |

新增模块名是推荐边界，不要求机械照搬；若 coder 合并文件，仍必须保持 `chat_backends`、`knowledge`、`claude transport` 三组所有权以及 D37 的依赖方向。

## 13. 实施顺序

1. **Slice 0：CLI/MCP compatibility fixture**：Ask tool exposure、Agent bypass、用户 MCP merge、reserved `nest`、init metadata、tool activity blocks。
2. **Slice 1：Knowledge core extraction**：从 `agent_tools.rs` 抽出 catalog/workspace/review/text merge；Rig adapter 回归全绿。
3. **Slice 2：DB 与 ChatTurn**：迁移、selection revision、activities、proposal claim/rebase、workspace health/manifest。
4. **Slice 3：Backend registry + composer**：descriptor commands、三 capsules、binding/new-session transfer、model snapshots、D56 observed-model projection、本地标题。
5. **Slice 4：MCP listener/adapter**：loopback auth、lease、transport、six tools、Stop/finalizer。
6. **Slice 5：Claude open runtime**：Ask/Agent arg matrix、external MCP merge、tool parser/activity persistence、Nest-first prompt。
7. **Slice 6：Reconciliation/review UI**：turn-end scan、incremental index、rebase/conflicted/resolved states、reindex_required。
8. **Slice 7：Full connection probe**：临时 Pack、两 turns、六工具证据、auto Apply、cleanup、partial mode status、`effective_model` 回流。
9. **Slice 8：Settings/UX hardening**：D54 Auto-detect 反馈、D55 行编辑器、Finalizing、session delete/shutdown/Vault gates、error copy、manual acceptance。

每个 slice 应独立通过相关 Rust/前端检查；后续 slice 不得复制前一 slice 的临时 adapter。Slice 0 未通过时先修正受支持 CLI contract，不得把未验证假设埋入后续实现。

## 14. 自动测试设计

### 14.1 Rust unit/component tests

- migration：旧 bound/unbound sessions、旧 messages 无 turn、旧 proposal statuses。
- selection：revision CAS、首次 binding 原子性、最近 session 默认、invalid model/unknown Backend。
- model options：Connection Report + 最近 20 个成功 ChatTurn effective model + Custom models 的顺序/去重；失败/取消/空值排除。
- capability：六项 schema、Ask/Agent catalog、active pack/protected/symlink/limits/permissions。
- effective view：staged > pending > disk，conflicted/resolved 不 overlay，pending search overlay。
- text merge：非重叠 clean、相邻/重叠、create/create、delete/modify、CRLF/UTF-8、already satisfied、重复 rebase。
- review：apply/rebase claim竞争、Approve-triggered re-review、crash recovery、rollback ownership。
- MCP：loopback/auth/Origin/Host/body limits、lease expiry、wrong session/mode、Stop cancellation。
- CLI：Ask/Agent args、model override、MCP merge/collision、stream tool blocks、redaction、Step 1 resume fallback。
- reconciliation：success/fail/cancel均扫描、incremental create/modify/delete、timeout → reindex_required、恢复清除。
- finalizer：success/error/Stop/delete/shutdown 幂等且释放 operation slot。

### 14.2 Frontend tests

- capsules 联动、unavailable reason、bound switch新建 session与 draft/focus迁移。
- stale revision、全局 busy、跨 tab Stop/跳转、Vault/Settings门禁。
- Tool Activity live upsert/history hydration、direct/Nest/external labels、sanitized expansion。
- Finalizing、interrupted、warning banner、Reindex recovery。
- Proposal pending/rebased/conflicted/resolved/applying/failed 和二次 Approve。
- deterministic Claude title、manual title保护、Unicode截断。
- Settings probe phases、connected-with-warning、Ask disabled、cleanup residue。
- Auto-detect 成功填 path并显示 version 行；编辑使旧结果失效；空路径失败显示 failure placeholder；过期 mutation response不覆盖新 draft。
- Custom models rows 的 hydrate/add/Enter-focus/delete/duplicate/normalize/keyboard/disabled 行为。
- Test 的未保存 `effective_model` 回流 Settings Detected models、配置指纹隔离、Save/成功 turn 的 descriptor 回流，以及不改写 Custom models/当前 selection。

### 14.3 Fake CLI integration

fixture 必须可脚本化输出 init/tool_use/tool_result/result、stderr 与 exit code，并记录 argv/stdin/MCP requests。覆盖：六个 Nest tools、原生 Bash/Edit/Write、外部 MCP、session ID-in-use resume、model switch、Stop process tree、终端结果后崩溃、reconciliation timeout。

### 14.4 回归检查

按仓库规范完整运行 Desktop UI 与 Rust 检查：

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

不得用仅运行新增测试替代全量 Desktop 回归；Step 2 改动跨越 DB、聊天、索引、Proposal 和 Settings。

## 15. Windows 手工验收

1. Save and connect 自动建立临时 Pack，两 turn 实际调用六个 Nest tools并完全清理。
2. 新 session 三 capsules工作；Claude Agent 为最近 session 时下一个新 session默认 Claude。
3. Ask 无 Bash/Edit/Write；Agent 可见并能执行 Bash，同时可调用 Nest MCP。
4. Claude 用 Nest tool 创建/修改/删除 Markdown，磁盘审批前不变，Approve 后落盘。
5. 同一 turn Nest stage 与 Bash 修改不同段落，Finalizing clean rebase并展示 current → merged Proposal。
6. 重叠修改进入 conflicted；不写 markers、不覆盖 Bash结果。
7. 原生 Read 只显示活动，不出现在 Sources；Nest read/search 生成 Sources。
8. 原生修改 active Pack 文件后 turn-end 可被下一 turn检索；Pack 外修改不进入索引。
9. Stop/错误后 stage消失、直接修改保留、activities持久化、索引协调完成或显示 reindex_required。
10. reconciliation timeout 后 Claude 原生 Agent仍可用，Nest Agent/Knowledge tools禁用；Reindex 成功恢复。
11. active turn期间删除 session、改 CLI/Vault、退出应用均符合 D53，且无遗留 Claude进程/lease。
12. 同名用户 `nest` MCP 被遮蔽并警告，其他外部 MCP 在 Agent可用、Ask不可见。
13. 清空 CLI Path 后执行 Auto-detect：成功时自动填入 resolved path并显示 CLI version；再次编辑 path 后检测行消失。让检测失败时显示手工输入 placeholder，且不会清掉已输入的无效路径。
14. Custom models 以行编辑器增删和键盘添加，保存/重开后顺序稳定，空行与重复项按 D55 规范化。
15. Test connection 返回的 `effective_model` 立即出现在 Settings 的 Detected models；未保存且路径不同的结果不污染 composer。Save 后该模型出现在 Model capsule并在重启后仍来自 Connection Report；一次成功 turn 返回新的 effective model 后该模型也进入选项，但 Custom models rows 和当前 selection均不被自动修改。

## 16. 完成定义

Step 2 只有在以下条件全部满足时完成：

- Slice 0 在目标 Windows Claude CLI 上验证 Ask 工具暴露、Agent bypass、用户 MCP merge、reserved `nest`、init metadata 和 tool stream；验证结果固化为 fixture/测试，而不是只保留人工观察。
- 新会话默认继承最近创建 session 的 Backend/Model，三个 capsule 与 selection revision/首次原子 binding 行为符合 D2、D6–D9、D35、D38。
- Claude Ask/Agent 的 CLI 与 MCP 启动参数严格符合 §8；Ask policy 不受支持时只禁用 Ask，Agent 可保持 connected-with-warning。
- Claude 能实际调用全部六个 Nest Knowledge Capabilities；search/read 形成真实 Sources，写入形成审批前不落盘的 Proposal。
- Claude 原生 Bash/Edit/Write 保持可用并显示 Direct Workspace Change Tool Activity，不出现虚假 Approve/Reject 或 Nest Citation。
- Stage、pending Proposal、Approve-time baseline 变化和 Direct Workspace Change 的组合均符合 D12、D41、D44、D52；clean rebase要求重新审阅，真正冲突不覆盖磁盘。
- ChatTurn 与 Tool Activity 在成功、失败、Stop、崩溃和重启恢复后保持可审计，敏感原始参数、credential 和完整输出不落库。
- Vault Reconciliation 在成功、失败和取消 turn 后执行；失败进入持久化 `reindex_required`，Reindex 能恢复 Nest Knowledge 与 Nest Agent。
- Save and connect 的两轮 probe 覆盖六个 Nest tools，能识别 native bypass、支持取消，并清理临时 Pack、session、proposal、index 和进程残留。
- active operation 对 session 删除、Claude Settings、Vault 切换和应用退出的门禁/清理符合 D53。
- Step 1 的 resume 自愈、Nest Agent RAG/权限/Proposal/标题、旧消息与旧会话迁移无回归。
- Auto-detect、Custom models 行编辑器和 `effective_model` 回流分别满足 D54–D56，并通过自动测试与第 15 节场景 13–15。
- 第 14 节自动测试、第 15 节 Windows 手工验收和仓库 sanity checks 全部通过。

## 17. Step 2 留白

- LLM Wiki ingestion/provenance/UI。
- Project Skill discovery/activation/packaging/execution UI；设计不得阻止后续接入。
- Pack sync/publish、Hub control 的实际 Agent tools；本步只保留 Domain Capability seam。
- Claude Agent SDK、原生逐工具审批、checkpoint/rewind。
- 多 session并发 ChatTurn、长驻 Claude process、跨设备 transcript同步。
- 常驻 Vault filesystem watcher、实时外部编辑同步。
- active Pack 外文档进入 Nest index、二进制/rename/move Knowledge tools。
- 自动 LLM conflict resolution、隐藏 merge turn、Proposal编辑器或 conflict markers。
- 动态安装第三方 Backend/plugin marketplace。
