# Claude Agent Step 2 设计

> 状态:草案,待检阅
> 范围:Step 2 全量——核心功能(两级选择、模型传参)+ Step 1 验收反馈的三项配置体验增强
> 前置:[Step 1 设计](./claude-agent-step1-design.md)(已实现并通过验收)
> 术语基线:[桌面端领域上下文](../../apps/desktop/CONTEXT.md)

## 1. 背景与总体结构

Step 1 交付了 Claude Agent 的完整接入:配置区、原子后端绑定、CLI 逐轮编排、
NDJSON 流式解析、四态连接状态(disabled / connected / last_connected /
unavailable,含持久化与断连重连按钮)。

Step 2 的目标由两部分组成,分两批交付:

| 批次 | 内容 | 性质 |
|------|------|------|
| **A. 配置体验增强** | Step 1 验收反馈的三项:auto-detect 结果反馈、custom models 行编辑器、检测模型回流 | 纯前端反馈层,零 schema 变更,先行交付 |
| **B. 核心功能** | 聊天框内**两级选择**(Agent → Model)、`claude_selected_model` 持久化与 `--model` 传参、会话级后端切换 | 既有规划主体,依赖批次 A 的行编辑器作为数据维护入口 |

批次 A 先行的原因:三个问题都是操作反馈缺失,不依赖批次 B 的任何基建;
而批次 B 的模型选择器直接消费批次 A 的行编辑器与回流数据。

## 2. 批次 A:配置体验增强

### A1. Auto-detect 结果反馈

现状(`ClaudeAgentSettingsSection.tsx` 的 `claudeDetect` mutation):

- 成功:仅把 `detection.resolved_path` 写入输入框 value;`ClaudeDetectionDto` 已经
  返回 `cli_version`(探测产出),但 UI 从未展示——版本信息被丢弃。
- 失败:仅一条 error toast;输入框 placeholder 恒为静态的
  `claude.exe · cli-wrapper.cjs · empty = auto-detect`,不随结果变化。
- 用户无法区分"检测成功了""检测失败但 CLI 确实不存在""检测失败因为路径写错"。

已验证底层链路正常(`node <wrapper> --version` 在本机输出 `2.1.238 (Claude Code)`,
resolver 能从 PATH 上的 `claude.cmd` 定位 wrapper);问题纯粹在反馈层。

交互规格:

| 检测结果 | 输入框 value | 输入框 placeholder | 输入框下方信息行 | 其他 |
|---------|-------------|--------------------|-----------------|------|
| 成功 | 填入 `resolved_path` | 不变 | `✓ Claude CLI 2.1.238 · node-script` | draft 标记 dirty(现状保留) |
| 失败(未找到) | 保持用户输入(通常为空) | 切换为 `Auto-detect found no Claude CLI — enter the path manually` | 红色 `✗ No Claude CLI found on PATH or npm locations` | toast 保留 |
| 进行中 | 不变 | 不变 | `Detecting…`(按钮 spinner 保留) | — |

实现要点:

- `detect.onSuccess`:新增 `detectionResult` state 存 `ClaudeDetectionDto`,
  在 Field 内渲染信息行(复用 `CheckCircle2`/`XCircle` 样式模式)。
- `detect.onError`:置 `detectFailed = true` 驱动 placeholder 切换;任何对
  输入框的手动编辑把 `detectFailed` 复位。
- placeholder 由常量改为三元:`detectFailed ? 未找到提示 : 默认提示`。
- 信息行同时展示 `spawn_strategy`(`direct` / `node-script`),解释
  "输入 shim、实际执行 wrapper"的解析行为。
- 检测信息行与连接报告区并存,互不覆盖。

### A2. Custom models 行编辑器

现状:单个 `Textarea`,一行一个模型 ID——无法阻止空行/重复,无行级操作,
与批次 B 选择器的离散条目心智不符。

交互规格:

- 一行一个受控 `Input`,行尾删除按钮;`0` 行时显示一行空输入框。
- 最后一行输入非空内容时自动追加新空行;另提供 `Add model` 按钮显式加行。
- 行内 trim 后与已有行重复:右侧显示 `Duplicate` 小字(不阻止输入,
  保存时后端规范化去重,幂等)。
- 空行在序列化时丢弃,不弹错误。

数据契约(不变):`AppSettings.claude_custom_models` 保持换行分隔 string
(Rust / DB / shared TS 零变更);前端 draft 层持有 `string[]`,
加载 `split("\n")`、保存 `filter(非空).join("\n")`。

组件:新建 `ClaudeModelsEditor`(settings 目录私有组件):
props `{ rows: string[]; onChange: (rows: string[]) => void }`。

### A3. 检测模型回流 Custom models

交互规格:

- `test` 与 `save` 成功且 `report.status === "connected" && effective_model`
  非空时:若该模型 ID 不在当前行列表(精确比较),自动追加为最后一行,
  toast 提示 `Added detected model: glm-5.3[1m]`。
- 触发于两个入口:Test connection(填入 draft,可删除)与
  Save and connect。
- 回流是一次性动作:用户随后手动删除该行是允许的,不监听不回写。
- `effective_model` 原样填入,保留 CLI 变体后缀(如 `[1m]`)。
- disabled 保存(无 model)不回流;回流只改 draft,需再点 Save 落库。

## 3. 批次 B:核心功能——两级选择与模型传参

> 本节吸收并取代旧草案(`claude-agent-settings-design.md` §9)的规划;
> 该草案降级为历史参考。

### B1. 交互设计

聊天 composer 区域(Ask/Agent 模式选择器旁)新增两级选择:

```
┌─ Agent(级 1)──────────────────────┐
│  [ Nest Agent              ▾ ]     │   ← 始终存在
│    [ Claude  (CLI 已连接)   ]      │   ← 仅连接可用时出现
└────────────────────────────────────┘
┌─ Model(级 2,随级 1 联动)─────────┐
│  Nest Agent 选中:                  │
│    [ Default (API)         ▾ ]     │   ← 唯一选项,指向 LLM 三件套
│  Claude 选中:                      │
│    [ CLI 默认              ▾ ]     │   ← 空值,不传 --model
│    [ glm-5.3[1m]           ]      │   ← 来自 claude_custom_models
│    [ claude-sonnet-4-5     ]      │      + 回流的 effective_model
└────────────────────────────────────┘
```

### B2. 级 1 — Agent 选择器

| 项 | 设计 |
|----|------|
| 选项 | `Nest Agent`(始终存在,默认)+ `Claude`(条件出现) |
| Claude 出现条件 | 连接状态为 `connected` 或 `last_connected`(与 Step 1 composer 门禁同源:复用 `claudeComposerGate` 的状态输入) |
| 未连接时 | 唯一选项 `Nest Agent`,选择器可视作不存在 |
| 数据落点 | **全局默认**:`AppSettings.claude_agent_enabled` 语义升级——从"新会话绑定 Claude"变为"选中的默认后端"。新增 `AppSettings.agent_backend: String`("builtin" \| "claude"),`claude_agent_enabled` 保留为连接使能开关(决定 Claude 选项是否可选),两者职责分离 |
| 选择时机 | 全局默认在 Settings/选择器改动即刻生效,**只影响新 unbound 会话**;已绑定会话不可切换(Step 1 不可变绑定原则) |
| 与门禁关系 | Claude 选中但连接失效时:unbound 会话沿用 Step 1 语义(阻止发送 + Reconnect 按钮);Claude-bound 会话只读。选择器显示当前生效后端并提示"应用于新对话" |

### B3. 级 2 — Model 选择器(随级 1 联动)

**Nest Agent 选中时**:唯一选项 `Default (API)`——指向现有 LLM 三件套
(`llm_base_url` / `llm_api_key` / `chat_model`)。无可切换项,等价于
指向 LLM 配置的静态标签(点击可跳转 Settings 的 LLM 分组)。

**Claude 选中时**:选项 = `CLI 默认`(空值,不传 `--model`)+
`claude_custom_models` 行列表(批次 A 的编辑器即其维护 UI)。

数据落点:新增 `AppSettings.claude_selected_model: String`(空 = CLI 默认),
serde default + KV 存储,零迁移。

### B4. 运行时传参

- `chat_runtime::run_claude` 构造 `ClaudeTurnRequest` 时读取
  `claude_selected_model`:非空则 turn_args 追加
  `--model <value>`;空则不传(CLI 默认)。
- `--model` 仅对 Claude 分支生效;Ask/Agent 模式对 Claude 无语义差异,
  级 2 不因模式变化。
- 传参失败(模型不被 CLI 识别)按 Step 1 既有错误分类处理
  (`ClaudeTurnError::CliError` → 用户提示)。

### B5. 数据与迁移汇总

| 字段 | 类型 | 默认 | 写入者 |
|------|------|------|--------|
| `agent_backend` | string ("builtin"/"claude") | "builtin" | 级 1 选择器(经 claude_save_settings 扩展) |
| `claude_selected_model` | string | ""(CLI 默认) | 级 2 选择器(同上) |

- `ClaudeSettingsRequest` 扩展两字段;`claude_save_settings` 白名单
  对应新增两个 KV 键。
- 旧库零迁移(serde default)。
- `chat_send` 的 unbound 绑定决策从 `claude_agent_enabled && proven` 改为
  `agent_backend == "claude" && claude_agent_enabled && proven`——
  toggle 关闭时即使 backend 字段为 claude 也不绑定(开关保持最终否决权)。

### B6. UI 落点

- 级 1/级 2 选择器放在 `MentionComposer` 的工具条(mode 选择器旁),
  仅在存在 unbound 当前会话或全局设置入口时渲染;Claude-bound 会话
  显示当前后端标识(只读)。
- Settings 的 Claude 分组同步加"默认 Agent"与"默认模型"两个下拉
  (与 composer 选择器读写同一字段,双向一致)。

## 4. 不做(延续既有决策)

- 模型列表自动枚举(隐私 + 本地路由不可靠)。
- 已绑定会话的后端/模型切换(不可变绑定原则)。
- Nest 知识能力(RAG/工具/MCP 封装)——Step 3 议题,本步不涉及。
- 会话级(而非全局)后端偏好 —— Step 1 已用"绑定即不可变"回答,
  不重开。
- 多 CLI 候选手动切换。

## 5. 实施顺序

1. **A1-A3**(纯前端,一批提交):行编辑器 → 检测反馈 → 回流。
2. **B5 数据层**:`agent_backend` + `claude_selected_model` 字段、
   save 白名单、`chat_send` 绑定决策改造(Rust,测试先行)。
3. **B4 运行时**:`--model` 传参(claude_cli turn_args 扩展 + 测试)。
4. **B6 UI**:composer 两级选择器 + Settings 双下拉;门禁集成测试。
5. 手工验收 + 回归。

每步先补测试再实现;批次 B 的 Rust 改动逐切片提交。

## 6. 验收清单(手工)

批次 A:

1. 清空 CLI path → Auto-detect → 输入框填入 wrapper 路径,下方出现
   `✓ Claude CLI 2.1.238 · node-script`。
2. Auto-detect 失败 → placeholder 变"未找到"提示 + 红色 ✗ 行;
   手动输入后 placeholder 复位。
3. Custom models:加三行、删中间一行、末行输入自动长新行、
   重复行出现 Duplicate;Save 后回读一致(去重后)。
4. Test connection 成功 → models 末尾出现 `glm-5.3[1m]` + toast;
   再次 Test 不重复追加;手动删除后不再自动回来。

批次 B:

5. 连接可用时 composer 出现两级选择器;断连时 Claude 选项消失,
   仅 Nest Agent。
6. 级 1 切到 Claude → 新会话首条消息绑定 Claude;切回 Nest →
   新会话走 Nest;已绑定会话不受影响(提示"应用于新对话")。
7. 级 2 选 `glm-5.3[1m]` → 发送消息,CLI 进程参数含
   `--model glm-5.3[1m]`(可从 NEST_DEBUG 日志验证);选 `CLI 默认`
   → 无 `--model`。
8. Settings 与 composer 的默认 Agent/模型双向同步。
9. `agent_backend=claude` 但 toggle 关闭 → 新会话仍绑定 Nest
   (开关否决权)。

## 7. 测试计划

批次 A(vitest):

- `ClaudeModelsEditor`:初始空行/末行自动追加/删除行/重复提示/
  序列化语义。
- detect 成功/失败的状态与 placeholder 复位;test 成功回流 +
  不重复追加;disabled 保存不回流。
- 保存 payload 的 `claude_custom_models` 仍为换行 string(契约回归)。

批次 B(Rust + 前端):

- Rust:turn_args 带/不带 `--model`;绑定决策真值表
  (agent_backend × enabled × proven);save 白名单含新键;
  旧 payload 反序列化兼容。
- 前端:级 1/级 2 联动矩阵(backend × status → 可选项)、
  选择器与 Settings 双向同步、门禁回归(§9.1 表加 model 维度)。

回归:两端 sanity checks 全绿;Step 1 的 105 前端测试与 191 Rust
测试无回归。

## 8. Step 3 留白

- Nest 知识能力(检索、staged 提案、protected paths)对 Claude 的
  MCP 封装。
- 风险工具治理(原生工具 vs Nest 工具并存/替换策略)。
- 长连接/SDK/进程复用方案。
- 会话历史跨后端的统一视图。
