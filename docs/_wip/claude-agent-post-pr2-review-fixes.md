# Claude Agent post-PR2 review fixes

> 状态：实现与全量验证完成
>
> 日期：2026-08-29
>
> 审查基线：PR1 merge `2210192`、PR2 merge `c908762`、release `c06d703`

## 目标

修复 PR1/PR2 合入后 `personal/dev` 新增 Claude Settings 与 Backend Selection 逻辑中的配置隔离、过期反馈和操作门禁问题，并清理同批审查发现的维护性问题。

## 行为修正

### 1. Custom model 状态按 CLI 配置隔离

- 持久化身份为 `configured CLI path + model ID`，同一模型在不同 CLI 路径下的测试结果可以同时保留。
- Settings 查询只投影当前 CLI Path draft 对应的行状态。
- Backend Descriptor 只使用当前已保存 CLI path 对应的失败状态过滤 Model options。
- 没有 path 指纹的旧状态视为 unknown，不再影响 composer；用户重新测试后生成带指纹的新状态。
- 保存 Custom models 时仍按 model ID prune 已删除模型的所有 path 状态。

### 2. CLI Path 编辑立即使旧连接反馈失效

- Test connection 结果只有在 `configured_cli_path` 与当前 draft path 匹配时才显示。
- 用户编辑 CLI Path 时立即清除旧 Test/Detection 反馈和旧默认模型行。
- 未保存 draft 的测试结果继续只在 Settings 中可见，不进入已保存配置的 Backend Descriptor。

### 3. Connection probe 期间即时门禁

- Auto-detect、Test connection、model Test、Save and connect 任一操作进行时，Claude enable、CLI Path、操作按钮和 Custom models 编辑器同步禁用。
- 本地 mutation 状态立即生效，不再依赖 SettingsPanel 的 operation polling 才进入 disabled 状态。
- 后端 app-wide operation slot 继续作为并发操作的最终保护。

### 4. 维护性清理

- Backend 可用性统一由 `isBackendUsable` 判定，避免 ChatPanel 与 capsule projection 漂移。
- `applySelection` 使用稳定 callback，补全 effect dependency。
- 删除未明确要求的解释性注释和重复的 Windows cfg 属性。

## 测试覆盖

- Rust：同一 model 在两个 CLI path 下的状态可以并存。
- Rust：其他 CLI path 的失败状态不会从当前 Descriptor 隐藏模型。
- React：编辑 CLI Path 后旧 Connected/default model 立即消失。
- React：model Test pending 时 Claude Settings 控件立即全部禁用。
- TypeScript：ready 与 last-verified 是唯一可用 Backend 状态。

## 验证结果

- Desktop UI：lint 通过（仅保留 5 条既有 warning），29 个测试文件、140 项测试通过，production build 通过。
- Desktop Rust：`cargo fmt --check`、clippy `-D warnings` 通过，276 项测试通过。
- `git diff --check` 通过。

## 交付边界

生产代码、共享类型和自动测试可进入下一条 `feat/*-clean` PR。本文及 `docs/_wip/**` 仅用于 `personal/dev` 的审查、验收与 distill，不进入上游 clean branch。现有用户文档描述的目标行为没有变化，因此本批不需要新增用户操作说明。
