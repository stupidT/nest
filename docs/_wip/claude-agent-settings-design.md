# 设计文档:Claude Agent 接入配置(设置页)

> **已取代：** 本文是早期设置页草案，其中 Step 1 的全局 `agent_backend`、CLI 工作目录、autosave 和聊天接入边界等决策，均已由 [Claude Agent Step 1 详细设计](./claude-agent-step1-design.md) 取代，不得作为 Step 1 编码依据。本文 §9 的 Agent → Model 两级选择规划仍可作为 Step 2 设计输入；Step 2 最终行为以届时的新文档为准。

| 项 | 值 |
|----|----|
| 状态 | 已被 Step 1 详细设计取代；仅 §9 作为 Step 2 参考 |
| 分支 | `feat/claude-agent-settings`(自 `personal/dev` 切出) |
| 阶段 | 阶段 1/2:配置面 + 连接测试 + **两级选择入口预留**;**不含**聊天接入 |
| 下一阶段 | 两级选择模式生效(Agent→Model 切换)+ CLI 流式适配(见 §9) |

---

## 1. 背景与动机

Nest 当前的聊天 agent 由 Rust 侧 rig + OpenAI 兼容 API 独占承载,用户必须在
Settings 中配置 `llm_base_url / llm_api_key / chat_model` 三件套。目标是仿照
Obsidian Claudian 插件的形态,允许用户改用本机 Claude CLI 作为聊天大脑——
Nest 不再是模型宿主,认证、模型路由全部交给 CLI 自身的环境。

本阶段只做"配置面":让用户在设置页录入 CLI 路径、验证连通性、维护自定义
模型列表,并为下一阶段的两级选择模式(Agent → Model,见 §9)预留全部
数据入口。**切换后端的实际生效(聊天走 CLI)不在本阶段**。

参照物:`G:\Projects\claudian`(本地 clone),重点是
`src/providers/claude/cli/findClaudeCLIPath.ts`(探测顺序)与
`src/providers/claude/ui/ClaudeSettingsTab.ts`(设置项形态)。

## 2. 范围

### In scope

1. `AppSettings` 新增 Claude 相关字段(持久化到现有 settings 键值表):
   `claude_cli_path`、`claude_custom_models`(模型列表),以及预留字段
   `agent_backend`
2. Rust 新模块 `claude_cli.rs`:CLI 探测(移植 Claudian 优先级)+ spawn 策略
3. 新 Tauri 命令 `claude_test_connection`:四层瀑布诊断
4. SettingsPanel 新增 "Claude Agent" 分组:
   - **Agent 选择器占位**(本阶段仅 "Nest Agent" 单选项,静态展示)
   - CLI 路径输入、连接测试按钮(含版本/当前模型显示)
   - **自定义模型列表**(多行文本,每行一个模型 ID)
5. 前后端类型(packages/shared)与 api.ts 封装

### Out of scope(显式排除)

| 项 | 理由 |
|----|------|
| Agent 切换的运行时生效 + Claude 选项出现 | 下一阶段(见 §9 两级选择模式);本阶段仅做数据预留与 UI 占位 |
| 模型选择器(级 2 下拉) | 下一阶段;本阶段先维护列表数据(`claude_custom_models`) |
| 模型列表枚举(读配置/打 API) | 已否决:隐私顾虑 + 本地路由场景不可靠(讨论记录见下) |
| safe mode / 权限档位 | 按既定决策,CLI 接入固定最严格配置,不做档位 |
| 聊天流式适配、ChatStreamEvent 桥接 | 下一阶段 |
| 自动起标题(title.rs)迁移 | 下一阶段 |
| 会话历史(--resume) | 下一阶段之后 |

### 决策记录

- **两级选择模式为既定方向**(§9):级 1 选 Agent(未连接 Claude CLI 时
  唯一选项为 Nest Agent),级 2 选模型(Nest Agent 仅支持 Default 即 API
  接入;Claude 支持从 custom models 列表选择)。本阶段完成全部数据入口
  预留:列表数据、连接状态信号、`agent_backend` 字段。
- **数据模型用列表而非单一模型覆写**:原设想的单 `claude_model` 字段无法
  承载级 2 的"列表中选择"语义,改为 `claude_custom_models`(换行分隔,
  仿 Claudian customModels),当前阶段即提供维护 UI。
- **不做模型枚举**:CLI 无 headless 枚举命令(实测 `claude model list` 会被当
  prompt 发送);读取 `~/.claude/settings.json` 打 `/v1/models` 虽在本机可行,
  但存在隐私问题且本地路由/网关用户可能查不到。替代方案:连接测试显示
  **CLI 实际生效的当前模型**(来自 stream-json 的 `system/init` 消息),
  其余模型用手工维护的 custom models 列表。
- **连接测试即 CLI 接入验证**:一次探测分四层(存在 → 可运行 → 认证 →
  端到端),同时验证了未来聊天 spawn 要走的同一条代码路径;其成功状态
  即下一阶段级 1 选择器中 "Claude" 选项可出现的判定信号。
- **本机实测数据**(设计依据):`claude -p "ok" --output-format stream-json
  --verbose --max-turns 1` 端到端约 3s,`--version` < 2s;
  stderr 存在 `[claude-code:unrecognized_model]` 之类的诊断噪声,
  成功与否**只能以 stdout 的 JSON 行为准**。
- **不使用 `--bare`**:`--bare` 会把认证收窄为 `ANTHROPIC_API_KEY`/
  `apiKeyHelper`,OAuth 登录用户会误判失败。测试用普通模式 + 临时 cwd。

## 3. 现状梳理(设置链路,改动点标注)

```
packages/shared/src/index.ts:401   AppSettings TS 类型        [改:加字段]
        │
apps/desktop/src/.../SettingsPanel.tsx
  ├─ EMPTY 常量(:56)               表单初值/回填兜底          [改:加字段]
  ├─ persistKey(:87)               防抖自动保存的脏检查        [自动包含新字段]
  ├─ 450ms debounce → api.settingsSet(:205)                   [不改]
  └─ GeneralGroup + Field 组件      分组 UI(:424 起,LLM 组:532) [改:加分组]
        │
apps/desktop/src/lib/api.ts:96-107 settingsGet/Set           [改:加测试命令]
        │
apps/desktop/src-tauri/src/commands/settings.rs
  ├─ settings_get(:15) / settings_set(:254)                   [改:校验/规范化新字段]
        │
apps/desktop/src-tauri/src/db.rs
  ├─ AppSettings struct(:15,serde default 兜底)               [改:加字段]
  ├─ get_settings(:529,显式 match key)                        [改:加分支]
  └─ save_settings(:576,pairs 数组)                           [改:加对]
        │
SQLite settings 表(键值对,无 schema 迁移)                    [不改]
```

新命令注册:lib.rs 的 `invoke_handler`(与 `settings_get` 同处追加)。

## 4. 总体设计

```
SettingsPanel                          Rust
┌─ Claude Agent 分组 ─────────────┐
│ [Agent 选择器(占位,单选项)]      │   claude_test_connection(cli_path?)
│ [CLI 路径输入框]                  │──► claude_cli::resolve(显式路径或探测)
│  留空 = 自动探测                  │──► 层2: spawn `--version`
│ [测试连接按钮]                    │──► 层3+4: spawn `-p ok --output-format
│  ✓ CLI 已找到 …(瀑布状态)        │        stream-json --max-turns 1`(30s 超时)
│ [自定义模型列表 Textarea]         │
└─────────────────────────────────┘   ◄── ClaudeConnectionReport
```

探测与 spawn 逻辑收敛在独立模块 `claude_cli.rs`,连接测试命令只是它的第一个
消费者;下一阶段聊天适配直接复用同一 `ClaudeExecutable` 解析。

## 5. 详细设计

### 5.1 数据模型

**Rust `db.rs::AppSettings` 新增**(全部带 serde default,旧库零迁移):

```rust
/// Path to the Claude CLI executable. Empty = auto-detect at use time.
#[serde(default)]
pub claude_cli_path: String,
/// Custom model IDs for the Claude agent selector, one per line.
/// Consumed by the level-2 model selector (next phase); maintained in
/// Settings this phase. Empty = no custom models.
#[serde(default)]
pub claude_custom_models: String,
/// Reserved for the two-level agent selector (next phase).
/// "builtin" (default) | "claude". No UI writes "claude" this phase.
#[serde(default)]
pub agent_backend: String,
```

**DB 持久化**:`get_settings` 的 match 加三个分支,`save_settings` 的 `pairs`
加三行,键名 `claude_cli_path` / `claude_custom_models` / `agent_backend`。

**`settings_set` 规范化**(commands/settings.rs,与现有 trim 风格一致):

```rust
settings.claude_cli_path = settings.claude_cli_path.trim().to_string();
// Normalize the model list: per-line trim, drop empty lines,
// dedupe preserving first-seen order.
settings.claude_custom_models = normalize_model_list(&settings.claude_custom_models);
settings.agent_backend = match settings.agent_backend.trim() {
    "" | "builtin" => "builtin".to_string(),
    "claude" => "claude".to_string(),
    other => return Err(AppError::msg(format!("Unknown agent backend: {other}"))),
};
```

不校验 CLI 路径存在性(允许先保存、稍后安装 CLI;存在性由测试按钮反馈)。
模型行不做字符集白名单——模型 ID 形态开放(含 `openai/gpt-4o`、
`glm-5.3[1m]` 等),合法性由下一阶段的实际 spawn 判定。

**TS 类型**(packages/shared/src/index.ts:401):

```ts
claude_cli_path: string;
claude_custom_models: string;
agent_backend: string;
```

SettingsPanel 的 `EMPTY` 同步加 `claude_cli_path: ""`、
`claude_custom_models: ""`、`agent_backend: "builtin"`。

### 5.2 CLI 探测模块 `claude_cli.rs`(新文件)

**公开接口**:

```rust
pub struct ClaudeExecutable {
    pub path: PathBuf,
    pub strategy: SpawnStrategy,
}
pub enum SpawnStrategy {
    Direct,          // .exe / unix 可执行文件
    NodeScript,      // .cjs / .js → 需要解析 node
    CmdShim,         // .cmd / .bat → 需要 cmd.exe /c
}

/// 优先级:显式路径 > 探测列表 > PATH 扫描。返回 None 表示未找到。
pub fn resolve(explicit: Option<&str>) -> Option<ClaudeExecutable>;

/// 按策略构造可 spawn 的 Command(已设置 stdin/stdout piped)。
pub fn build_command(exe: &ClaudeExecutable, args: &[&str])
    -> std::process::Command;   // tokio::process::Command 由调用方包一层
```

**探测顺序**(移植 Claudian findClaudeCLIPath.ts:189-223):

Windows:
1. `%USERPROFILE%\.claude\local\claude.exe`
2. `%LOCALAPPDATA%\Claude\claude.exe`
3. `%ProgramFiles%\Claude\claude.exe`、`%ProgramFiles(x86)%\Claude\claude.exe`
4. `%USERPROFILE%\.local\bin\claude.exe`
5. npm 全局包入口:`%APPDATA%\npm\node_modules\@anthropic-ai\claude-code\`
   下的 `cli-wrapper.cjs` / `cli.js`(NodeScript)
6. PATH 逐项扫描:该项下 `claude.exe`(Direct)→
   `node_modules\@anthropic-ai\claude-code\cli-wrapper.cjs`(NodeScript)
7. PATH 逐项扫描:`claude.cmd`(CmdShim)——最后手段

macOS / Linux:
`~/.claude/local/claude`、`~/.local/bin/claude`、`/usr/local/bin/claude`、
`/opt/homebrew/bin/claude`、`~/.volta/bin/claude`、
`{npm prefix -g}/lib/node_modules/@anthropic-ai/claude-code/cli.js`,
最后 PATH 扫描 `claude`。

**显式路径**:非空时直接采用(必须存在且是文件,否则报错;按扩展名归类
SpawnStrategy;无扩展名的 Unix 脚本按 Direct 处理)。

**本机验证**:用户 npm 全局目录为 `G:\Apps\nodejs\node_global`(自定义
prefix),第 6 步 PATH 扫描会命中其下的 `claude.cmd` → CmdShim,
`cmd /c` 方式已实测 stream-json 输出正常。

**Claudian 的坑位注释同步适用**:无扩展名 `claude` 是 npm 的 POSIX sh shim,
Windows 下不可直接 spawn,必须跳过(跳到 .cmd 分支或包入口)。

### 5.3 连接测试命令

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeConnectionReport {
    pub cli_found: bool,
    pub cli_path: Option<String>,     // 实际使用的可执行文件路径
    pub spawn_strategy: String,       // "direct" | "node-script" | "cmd-shim"
    pub version: Option<String>,      // 层2 产物,如 "2.1.238"
    pub auth_ok: bool,
    pub model: Option<String>,        // system/init 的 model 字段
    pub e2e_ok: bool,
    pub e2e_latency_ms: Option<u64>,
    pub error: Option<String>,        // 分层错误 + 修复建议
}

#[tauri::command]
pub async fn claude_test_connection(
    cli_path: Option<String>,         // 来自输入框当前值,空 = 探测
) -> AppResult<ClaudeConnectionReport>;
```

**四层瀑布**(任一层失败即停止,`error` 携带该层信息):

| 层 | 动作 | 成功判据 | 典型失败 → 提示语 |
|----|------|---------|------------------|
| 1 存在 | `resolve(explicit)` | Some(exe) | 未找到 → "未找到 Claude CLI,请安装或填写完整路径(claude.ai/code)" |
| 2 可运行 | spawn `--version`,10s 超时 | 退出码 0,stdout 解析首 token 为版本 | 非零退出 → 附 stderr 尾部(截 500 字符) |
| 3+4 端到端 | spawn `-p "ok" --output-format stream-json --verbose --max-turns 1`,30s 超时 | stdout 出现 `"type":"result"` 且 `subtype:"success"` | 认证失败(stderr 含 "auth"/"login"/"credential"/"401")→ "CLI 未登录,请先在终端运行 claude 完成登录";其余 → 附 stderr 尾部 |

**spawn 细则**:

- **cwd = `std::env::temp_dir()`**:避免 CLI 在当前 repo 下做 CLAUDE.md
  发现、写 `~/.claude/projects/<cwd-hash>` 会话状态污染 Nest 仓库
- **环境变量:全量继承**——这是设计核心,认证、网关、代理全部走 CLI
  自己的配置(Nest 零感知,也规避了读用户配置的隐私问题)
- **stdout 逐行读、逐行 `serde_json::from_str`**:非 JSON 行(CLI 横幅等)
  忽略;`system/init` 取 `model` 与 `claude_code_version`;
  `result` 取 `subtype` 判端到端成功;其余消息类型跳过
- **stderr 只收集不判失败**(实测成功运行也有诊断噪声),仅在第 3+4 层
  失败时用于错误分类
- **超时杀进程树**:Windows 下 `taskkill /T /F /PID`;Unix 下
  `kill(child.id())` + `killpg`。tokio 超时后必须确保子进程不残留
- **不使用 `--bare`**(见决策记录)
- 模型字段原样展示(含 `[1m]` 这类 CLI 变体后缀),不做解释性改写

**成本披露**:该测试执行一次真实 completion(实测约 1-4k tokens 含系统
提示,本机 $0.02)。按钮文案明确写"将发送一次最小请求"。

### 5.4 前端 UI

SettingsPanel 在现有 LLM 分组(:532,Bot 图标)之后新增一组
`GeneralGroup icon={Terminal} title="Claude Agent"`(图标用 lucide 的
`Terminal`,实现时定夺):

```
Claude Agent
├─ Field: Agent(两级选择的级 1 占位)
│    <Select disabled value="builtin">
│      <option>Nest Agent</option>          // 本阶段唯一选项,静态展示
│    </Select>
│    hint: "Claude 选项将在聊天接入阶段开放(需先通过连接测试)"
│    // 选中值绑定 form.agent_backend;本阶段 UI 不提供切换手段,
│    // 数据字段先行落库,下一阶段替换为可切换下拉(见 §9)
│
├─ Field: CLI 路径
│    description: "留空自动探测。填写 Claude CLI 可执行文件路径,
│                  例如 Windows 的 claude.exe 或 npm 全局包入口。"
│    <Input value={form.claude_cli_path} ... />   // 走通用 update() 自动保存
│    探测结果回显(见下)
│
├─ Field: 连接测试
│    <Button onClick={testClaude.mutate()}>测试连接</Button>
│    瀑布状态区(成功态):
│      ✓ CLI 已找到: G:\Apps\nodejs\node_global\claude.cmd (cmd-shim)
│      ✓ 版本: 2.1.238
│      ✓ 认证有效,当前模型: glm-5.3[1m]  (端到端 3.0s)
│    失败态: 红色 X + 分层错误信息 + 修复建议
│    测试中: Loader 动画 + 禁用按钮(参照 Hub 测试按钮 :454 的现有模式)
│
└─ Field: 自定义模型列表(可选,两级选择的级 2 数据源)
     description: "每行一个模型 ID(如 glm-5.3、claude-sonnet-4-5)。
                   聊天接入后,模型选择器将从此列表生成选项。"
     <Textarea rows={4} value={form.claude_custom_models} ... />
     // 复用 @/components/ui/textarea(PublishPackDialog 已在用)
```

**实现要点**:

- 复用 Hub 连接测试的既有模式(`hubTestResult` state + useMutation,
  SettingsPanel:132/:454),新开 `claudeTestResult` state +
  `testClaudeConnection` mutation,不抽公共组件(两组语义不同)
- 测试命令参数传**输入框当前值**(含未保存的编辑),而非 `form` 里已
  保存值——用户改完路径立即可测
- 新字段自动参与 `persistKey` 脏检查与 450ms 自动保存,无需额外逻辑
- Agent 选择器占位用现有 `@/components/ui/select`(disabled 态),不引入
  新组件;它的存在让下一阶段的升级是"启用 + 加选项"而非"改布局"
- 分组 description 注明"接入后 Nest 聊天将改用 Claude CLI(功能开发中)"
  ——管理预期,避免用户以为切换已生效

### 5.5 i18n

现状:`display_language` 仅 `"en"`,LLM 分组的 help 文案是内联英文。
**新文案全部走 `t()` 键**(settings.claude.*),值用英文,与现有键位风格
一致(lib/i18n.tsx)。为未来多语言留好键位,但不新增语言。

## 6. 边界情况与风险

| 风险 | 处理 |
|------|------|
| Windows `.cmd` shim 的 stdio 流 | Rust 显式 `cmd.exe /c` spawn(非 shell 注入路径);本机已实测 stream-json 正常。Claudian 的 "breaks stdio streaming" 指 Node SDK 不带 shell 的场景,不适用于我们的显式 cmd 包装 |
| 用户机器无 node 但有 .cmd shim | CmdShim 策略不依赖我们解析 node(CLI 自身是 node 脚本,由 shim 内部解析);NodeScript 策略才需要 PATH 上的 node |
| CLI 卡死不退出 | 层 2/层 4 均有超时 + 杀进程树;UI 按钮防重复点击 |
| 网关模型 ID 不被 CLI 识别 | 不影响测试判据(实测本机有 unrecognized_model 噪声但 result 仍 success);仅当 result 缺失/subtype 非 success 才判失败 |
| 旧版本库无新键 | serde default + settings 表键值对,天然兼容;老二进制读新库同理 |
| 测试请求花钱 | 文案披露;prompt 固定 "ok" + `--max-turns 1`,成本上界受控 |
| `~` 展开的显式路径 | Windows/macOS 均支持 `~` 前缀 → Rust 侧手动展开(dirs::home_dir) |
| 并发调用测试命令 | 每次 spawn 独立进程,天然幂等;UI 层再加禁重入 |

## 7. 测试计划

**Rust 单元测试**(`claude_cli.rs` 内 `#[cfg(test)]`,遵循 agent.rs 既有模式;
`normalize_model_list` 的测试放 `db.rs` 或 commands/settings.rs 测试模块):

1. `spawn_strategy_by_extension`:`claude.exe→Direct`、`cli.js→NodeScript`、
   `claude.cmd→CmdShim`、无扩展名 unix 路径→Direct
2. `explicit_path_validation`:不存在/是目录 → None;存在文件 → Some
3. `parse_version_output`:`"2.1.238 (Claude Code)\r\n"` → `Some("2.1.238")`
4. `parse_stream_lines`:fixture JSON 行(init + result)→ 报告字段正确;
   非 JSON 行被忽略;仅 init 无 result → `e2e_ok=false`
5. `classify_auth_error`:含 "Please run /login" 的 stderr → 认证类提示
6. `~` 展开:home 前缀路径展开正确
7. `normalize_model_list`:换行/空格混入 → 逐行 trim;空行剔除;
   重复项保序去重;`""` → `""`
8. `agent_backend_validation`:`""`/`"builtin"` → `"builtin"`;
   `"claude"` 透传;`"foo"` → Err

**手动验收清单**(本机,`tauri-dev.cmd` 起桌面端):

- [ ] 路径留空 + 测试 → 四层全绿,路径显示 `G:\Apps\nodejs\node_global\claude.cmd`,版本 2.1.238,模型 glm-5.3 系列
- [ ] 填入不存在的路径 + 测试 → 层 1 红色错误
- [ ] 填入有效路径(如直接指 claude.cmd)→ 同自动探测结果
- [ ] 模型列表输入多行(含空行/重复行/首尾空格)→ 保存后回读已规范化
- [ ] Agent 选择器占位显示 "Nest Agent",不可交互
- [ ] 旧设置(LLM 三件套)不受影响,聊天行为不变
- [ ] 断网测试 → 层 4 超时/网络错误提示,无残留进程(tasklist 查 claude/node)

**Sanity checks**(AGENTS.md):

```powershell
cd apps/desktop; npm run lint; npm test; npm run build
cd apps/desktop/src-tauri; cargo fmt --check; cargo clippy --all-targets -- -D warnings; cargo test
```

## 8. 涉及文件清单

| 文件 | 动作 |
|------|------|
| `packages/shared/src/index.ts` | 改:AppSettings 加 3 字段 + ClaudeConnectionReport 类型 |
| `apps/desktop/src-tauri/src/db.rs` | 改:AppSettings 字段 + get/save 分支 |
| `apps/desktop/src-tauri/src/claude_cli.rs` | **新**:探测 + spawn 策略 + 流解析 + 测试逻辑 |
| `apps/desktop/src-tauri/src/commands/mod.rs` + `lib.rs` | 改:注册 `claude_test_connection` |
| `apps/desktop/src-tauri/src/commands/settings.rs` | 改:set 规范化新字段 |
| `apps/desktop/src/lib/api.ts` | 改:`claudeTestConnection` 封装 |
| `apps/desktop/src/components/settings/SettingsPanel.tsx` | 改:Claude Agent 分组 |
| `apps/desktop/src/lib/i18n.tsx` | 改:settings.claude.* 键 |

`claude_test_connection` 命令放 `commands/settings.rs` 还是独立
`commands/claude.rs`:倾向**独立文件**(测试逻辑与 settings CRUD 无关,
且下一阶段会膨胀成聊天后端),实现时若发现耦合再并入。

## 9. 下一阶段规划:两级选择模式(Agent → Model)

> 本节是既定方向的完整设计,下一阶段(聊天接入阶段)实施。本阶段已完成
> 其全部数据入口预留(见 9.4),届时升级形态是"启用 + 接线",无 schema
> 迁移、无布局重构。

### 9.1 交互设计

```
┌─ Agent(级 1)──────────────────────┐
│  [ Nest Agent              ▾ ]     │   ← 始终存在,默认选中
│    [ Claude  (CLI 已连接)   ]      │   ← 仅连接测试通过后出现/可选
└────────────────────────────────────┘
┌─ Model(级 2,随级 1 联动)─────────┐
│  Nest Agent 选中:                  │
│    [ Default (API)         ▾ ]     │   ← 唯一选项,不可切换
│  Claude 选中:                      │
│    [ CLI 默认              ▾ ]     │   ← 空值,不传 --model
│    [ glm-5.3               ]      │   ← 来自 claude_custom_models
│    [ claude-sonnet-4-5     ]      │      列表,逐行生成选项
└────────────────────────────────────┘
```

### 9.2 级 1 — Agent 选择器

| 项 | 设计 |
|----|------|
| 选项 | `Nest Agent`(始终存在,默认)+ `Claude`(条件出现) |
| Claude 出现条件 | CLI 连接状态 = 已连接:**本阶段连接测试成功** 且 运行时 `claude_cli::resolve()` 能找到 CLI(二者取与) |
| 未连接时 | 唯一选项 `Nest Agent`(此时选择器可视作不存在,不产生噪音) |
| 数据落点 | `AppSettings.agent_backend`:`"builtin"` \| `"claude"`(**本阶段已预留字段**,默认 builtin) |
| 连接状态存储 | 不落库——每次打开设置页时由 `claude_test_connection` 的轻量结果(或下一次聊天 spawn 的成败)即时判定;避免"上次连上、这次拔了"的陈旧状态 |

### 9.3 级 2 — Model 选择器(随级 1 联动)

**Nest Agent 选中时**:唯一选项 `Default (API)`——指向现有 LLM 三件套
(`llm_base_url` / `llm_api_key` / `chat_model`,Settings 的 LLM 分组)。
没有可切换项;级 2 在此形态下等价于一个指向 LLM 配置的静态标签。

**Claude 选中时**:选项 = `CLI 默认`(空值,spawn 时不传 `--model`)+
`claude_custom_models` 列表逐行生成的条目(**本阶段已维护的数据**)。

数据落点:新增 `AppSettings.claude_selected_model: String`(空 = CLI 默认)。
本阶段**不**建此字段——它的语义随选择器诞生,下一阶段加(settings 键值表
+ serde default,零迁移)。

### 9.4 本阶段已完成的入口预留清单

| 预留 | 承载物 | 下一阶段消费方式 |
|------|--------|-----------------|
| 级 1 数据落点 | `agent_backend` 字段(serde default "builtin",本阶段 UI 固定 builtin) | 选择器启用后读写该字段 |
| 级 2 选项来源 | `claude_custom_models` 列表 + 设置页 Textarea 维护 UI | 模型下拉逐行解析生成选项 |
| 级 1 门控信号 | `claude_test_connection` 四层报告 | `e2e_ok && cli_found` → Claude 选项出现 |
| 运行时 spawn 基础 | `claude_cli.rs` 的 `resolve` / `build_command` | 聊天适配直接复用,加 `--model {selected}` |
| UI 布局槽位 | Claude Agent 分组顶部的 Agent 选择器占位(单选项静态展示) | 替换为可切换下拉,加 Claude 选项与级 2 联动 |

### 9.5 下一阶段工作清单

1. `AppSettings` 加 `claude_selected_model` 字段(空 = CLI 默认)
2. Agent 选择器从占位升级为可切换:启用交互、按连接状态追加 Claude 选项
3. Model 选择器(级 2)落地:联动逻辑 + `claude_custom_models` 解析
4. `chat_send` → `ChatBackend::Builtin | Claude` 分支(agent.rs:99 接缝),
   Claude 分支走 CLI 流式适配(stream-json → ChatStreamEvent 桥接)
5. `agent_backend = "claude"` 但 CLI 不可用时的降级策略(报错提示重测,
   或回落 builtin——实现时定夺)
6. `title.rs` 自动起标题迁移(Claude 后端下由 spawn 顺带产出)
7. 选定模型作为 `--model` 参数传入;`claude_selected_model` 为空则不传

**选择器位置**:本规划默认放在 Settings 的 Claude Agent 分组(全局默认,
与现有分组结构一致)。可选扩展:聊天 composer 内的会话级切换(仿现有
ask/agent mode 的 per-session 模式,`ChatSession` 加 backend 字段)——
是否需要留待下一阶段设计评审,不改变本阶段预留内容。

### 9.6 明确不做(延续既有决策)

- 模型列表自动枚举(隐私 + 本地路由不可靠,见决策记录)
- safe mode / 能力档位(固定最严格配置)
- 每 chat 会话独立 agent 记忆(`--resume` 之后的议题)
