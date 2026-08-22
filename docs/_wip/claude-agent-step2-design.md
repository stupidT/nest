# Claude Agent Step 2 设计:配置体验优化(第一批)

> 状态:草案,待检阅
> 范围:Step 1 验收反馈的三个易用性问题;Step 2 主体(两级选择器、知识能力封装)另行设计
> 前置:[Step 1 设计](./claude-agent-step1-design.md)(已实现,切片 1-8 落地)
> 术语基线:[桌面端领域上下文](../../apps/desktop/CONTEXT.md)

## 1. 背景

Step 1 交付了 Claude Agent 的完整接入:Settings 配置区(toggle / CLI path / custom models /
detect / test / save and connect)、原子后端绑定、CLI 逐轮进程编排、NDJSON 流式解析、
连接探测。手工验收中功能链路可用,但暴露出三个配置体验问题——共同点是
**操作完成后用户得不到足够的结果反馈**,需要靠猜或重试。

本批优化目标:让 Claude 配置区的每次操作都有可见、可理解的结果。

## 2. 问题清单(Step 1 验收反馈)

### P1 Auto-detect 没有可见结果

现状(`SettingsPanel.tsx` 的 `claudeDetect` mutation):

- 成功:仅把 `detection.resolved_path` 写入输入框 value;`ClaudeDetectionDto` 已经
  返回 `cli_version`(探测产出),但 UI 从未展示——版本信息被丢弃。
- 失败:仅一条 error toast;输入框 placeholder 恒为静态的
  `claude.exe · cli-wrapper.cjs · empty = auto-detect`,不随结果变化。
- 用户无法区分"检测成功了""检测失败但 CLI 确实不存在""检测失败因为路径写错"。

已验证底层链路正常(`node <wrapper> --version` 在本机输出 `2.1.238 (Claude Code)`,
resolver 能从 PATH 上的 `claude.cmd` 定位 wrapper);问题纯粹在反馈层。

### P2 Custom models 的多行文本框不易输入

现状:单个 `Textarea`,一行一个模型 ID。

- 无法阻止空行、行内空格、重复项——用户只能在保存后回读才能发现规范化结果;
- 无每行的增删操作,清空中间一行要手动选中删除;
- 与 Step 2 主体的"模型二级选择"心智不符:选择器消费的是离散条目,编辑器却
  是自由文本。

### P3 Test connection 检测到的模型没有回流

现状:`claudeTestConnection` 成功后报告里已含 `effective_model`(CLI 当前实际生效
模型,如 `glm-5.3[1m]`),但只展示在状态区,不进入 Custom models。用户明明想让
选择器覆盖该模型,还得手动抄一遍。

## 3. 方案设计

### 3.1 Auto-detect 结果反馈(修 P1)

交互规格:

| 检测结果 | 输入框 value | 输入框 placeholder | 输入框下方信息行 | 其他 |
|---------|-------------|--------------------|-----------------|------|
| 成功 | 填入 `resolved_path` | 不变 | `✓ Claude CLI 2.1.238 · node-script` | draft 标记 dirty(现状保留) |
| 失败(未找到) | 保持用户输入(通常为空) | 切换为 `Auto-detect found no Claude CLI — enter the path manually` | 红色 `✗ No Claude CLI found on PATH or npm locations` | toast 保留 |
| 进行中 | 不变 | 不变 | `Detecting…`(按钮 spinner 保留) | — |

实现要点:

- `claudeDetect.onSuccess`:除填 `cliPath` 外,新增 `detectionResult` state 存
  `ClaudeDetectionDto`,在 Field 内渲染信息行(复用 Hub 测试按钮下方
  `hubTestResult` 的样式模式:`CheckCircle2`/`XCircle` + 文案)。
- `claudeDetect.onError`:置 `detectionResult = null` 并置 `detectFailed = true`,
  驱动 placeholder 切换。任何对输入框的手动编辑把 `detectFailed` 复位
  (placeholder 回到默认)。
- placeholder 由常量改为三元:`detectFailed ? 未找到提示 : 默认提示`。
- 信息行同时展示 `spawn_strategy`(`direct` / `node-script`),解释
  "输入 shim、实际执行 wrapper"的解析行为(Step 1 设计 §7.2 的 UI 承诺)。
- 检测成功后的信息行与后续 Test connection 的报告区并存,互不覆盖
  (一个说明"用什么启动",一个说明"连没连上")。

### 3.2 Custom models 行编辑器(修 P2)

交互规格:

- 一行一个受控 `Input`,行尾一个删除按钮(`X` icon button);`0` 行时显示
  一行空输入框。
- 最后一行输入非空内容时自动追加新的空行(常见 tags-editor 模式);
  另提供 `Add model` 按钮显式加行,照顾清空后重新开始的场景。
- 行内 trim 后与已有行重复:该行右侧显示 `Duplicate` 小字提示(不阻止输入,
  保存时后端规范化去重,与现状一致)。
- 不做空行阻止——空行在序列化时被丢弃,不弹错误。

数据契约(不变):

- `AppSettings.claude_custom_models` 在 Rust / DB / shared TS 中保持
  **换行分隔 string**。零 schema 变更、零 Rust 改动。
- 前端在 draft 层持有 `string[]`,加载时 `split("\n")`,保存时
  `filter(非空).join("\n")`;`claudeDirty` 比较基于 join 后的串,与后端
  规范化语义一致(后端再 trim/去重一次,幂等)。

组件形态:

- 新建 `ClaudeModelsEditor`(SettingsPanel 内部私有组件即可,不进 ui/):
  props `{ rows: string[]; onChange: (rows: string[]) => void }`。
- 替换现有 `Textarea` 的位置;`Field` 的 label/description 沿用
  (description 改为 "One model ID per row.")。

### 3.3 检测模型回流 Custom models(修 P3)

交互规格:

- `claudeTest` 与 `claudeSave` 成功且 `report.connected && report.effective_model`
  非空时:若该模型 ID 不在当前 models 行列表(精确字符串比较,不做模糊匹配),
  自动追加为最后一行,并 toast/info 提示 `Added detected model: glm-5.3[1m]`。
- 追加触发于**两个入口**:Test connection(未保存也会填入 draft,用户看得见、
  可删除)与 Save and connect(保存后回读刷新时同样补进 draft)。
- 用户随后手动删除该行是允许的——回流是一次性动作,不监听不回写。
- `effective_model` 原样填入,保留 CLI 变体后缀(如 `[1m]`),不做改写
  (与连接报告展示口径一致)。

边界:

- 若 report 来自 Save(disabled 路径无 model),不回流。
- 回流只改 draft 行数组;dirty 判定照常生效(用户需要再点 Save 才落库,
  与 P1 的 detect-填路径行为一致)。

## 4. 本批不做

- 不改 Rust 侧任何代码与存储 schema(三个问题全部是前端反馈层)。
- 不做聊天框内 Agent/模型二级选择(Step 2 主体,沿用
  [旧草案 §9](./claude-agent-settings-design.md) 的两级规划,届时行编辑器
  即其数据维护 UI)。
- 不做模型自动枚举(隐私 + 本地路由不可靠,既有决策,不重开)。
- 不做多 CLI 候选的手动切换(detect 返回首个探测成功者,维持现状)。
- 不把 custom models 迁移为 `string[]` 类型(避免一次 Rust/DB/前端三端联动,
  收益仅是省掉 join/split)。

## 5. 验收清单(手工)

1. 清空 CLI path → Auto-detect → 输入框填入 wrapper 路径,下方出现
   `✓ Claude CLI 2.1.238 · node-script`。
2. 断开 PATH(临时改环境或填入不存在路径)→ Auto-detect → placeholder 变为
   "未找到"提示,下方红色 `✗` 行;手动输入任意字符后 placeholder 复位。
3. Custom models:加三行、删中间一行、最后一行输入后自动长出新行、
   输入重复行出现 Duplicate 提示;Save 后回读与显示一致(去重后)。
4. Test connection 成功 → custom models 末尾出现 `glm-5.3[1m]`,
   toast 提示 Added;再次 Test 不重复追加。
5. Save and connect 成功 → 同样回流;随后手动删除该行 → 再 Test 不再自动回来
   (已存在判断按当前 draft)。

## 6. 测试计划

前端(vitest,参照 `SettingsPanel` 现有测试模式;如无组件测试基建则新增
`ClaudeModelsEditor.test.tsx` 覆盖纯组件部分):

- `ClaudeModelsEditor`:初始空行/末行输入自动追加/删除行/重复提示/
  onChange 序列化语义。
- detect 成功 → `cliPath` 与信息行状态;detect 失败 → placeholder 状态与复位。
- test 成功 → models 行追加 + 不重复追加;disabled 保存不回流。
- 保存 payload 的 `claude_custom_models` 仍为换行 string(契约回归)。

回归:本批改动后 `npm run lint && npm test && npm run build` 全绿;
Rust 侧零改动零重编译。

## 7. Step 2 主体留白(与本批的关系)

以下仍在 Step 2 主体设计时定义,本批不预实现:

- 聊天框一级 Agent 选择 / 二级模型选择(消费本批的 custom models 行数据)。
- Nest 知识能力(检索、staged 提案)对 Claude 的 MCP 封装。
- `claude_selected_model` 持久化字段与 `--model` 传参。
- 风险工具治理(原生工具 vs Nest 工具并存策略)。

本批交付的行编辑器与回流行为是选择器的数据维护入口:选择器上线时,
custom models 里的每一行即成为二级下拉的一个选项,`effective_model` 回流
保证"当前实际在用的模型"永远可选。
