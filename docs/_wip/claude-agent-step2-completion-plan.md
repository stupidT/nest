# Claude Agent Step 2 完成计划

> 状态：实现收口清单
>
> 适用分支：`feat/claude-agent`
>
> 来源：基于当前实现相对 `46d292c` 的 Standards/Spec review
>
> 本文是临时 WIP，不替代 `claude-agent-step2-design.md`，完成后不得进入上游 clean branch

## 1. 当前结论

当前实现已经能够在 Windows Nest 应用中连接 Claude CLI，并完成连续会话、Backend/Model/Mode 选择、Nest MCP 六工具接入、Sources、Tool Activity、Proposal 审批及基础三方归并。现状可以作为 Step 2 的本地演示和集成里程碑，但还不能标记为 Step 2 完成。

剩余工作集中在五个方面：Vault Reconciliation、turn 内二次归并、Proposal claim 与崩溃恢复、真实端到端 connection probe，以及通用 Backend/lifecycle 架构收口。

### 1.1 实施状态

- [x] Vault manifest、等待式 Reindex 与 degraded-mode capability gate
- [x] staged change 与 Direct Workspace Change 二次归并及大文件安全边界
- [ ] Proposal claim、apply journal 与启动恢复
- [ ] 完整 Claude 两轮六工具 connection probe
- [ ] Backend Descriptor、Knowledge effective view 与 app-wide operation slot
- [ ] 全量回归、Windows 手工验收与 Step 2 最终收口

## 2. P0：Vault Reconciliation 与 Reindex 闭环

### 2.1 实现内容

- 为 active Knowledge Packs 持久化可信的 Markdown manifest，至少保存规范化路径和内容摘要。
- turn-end、启动、Vault 切换和显式 Reindex 时，对比上次 manifest，识别新增、修改和删除。
- 对变化执行增量索引；无法信任 manifest 或变化量过大时允许全量重建。
- reconciliation 和显式 Reindex 必须等待目标 index generation 真正完成，不能在 `indexing::schedule` 返回后立即宣告成功。
- 索引或 reconciliation 失败时持久化 `reindex_required=true`、稳定 reason 和时间。
- 只有索引成功且 pending proposals reconciliation 完成后才能清除 `reindex_required`。
- `reindex_required` 时只关闭 Nest Knowledge Capability 和依赖它的 Nest Agent；Claude 原生工具与 External MCP 仍可运行。
- ChatTurn 持久化 `workspace_reconciliation_failed` warning，不能把 Claude 已完成的原生修改伪装成回滚。

### 2.2 验收

- Claude 用 Bash 创建、修改、删除 active Pack Markdown 后，下一轮 search/read 得到最新结果。
- 已索引文件的内容变化和删除不会漏检。
- 重建仍在运行时 workspace 不会提前恢复 healthy。
- 重建失败后 banner 和 Knowledge 门禁保持，重试成功后自动恢复。

## 3. P0：staged change 与 Direct Workspace Change 二次归并

### 3.1 实现内容

turn finalization 对每条 staged path 使用统一文本归并 seam：

```text
base     = stage 捕获的 old_content
proposed = Nest MCP staged content
current  = turn-end 磁盘内容
```

- `current == base` 时按原 proposal 完成。
- 非重叠变更执行确定性 clean merge，生成 `current -> merged` 的 rebased proposal，并要求重新审阅。
- 重叠变更生成 conflicted proposal，不覆盖磁盘。
- create/delete 与外部 create/delete 组合必须有明确矩阵。
- `finish_staged` 的归并或校验错误必须传播，禁止 `unwrap_or_default`。
- staged paths 的最终校验必须发生在 assistant/proposal 最终事务之前。
- 三方归并需限制计算规模；超过安全行数或矩阵预算时安全转为 conflict，不能因合法的 256 KiB 文档耗尽内存。

### 3.2 验收

- 同一 turn 中 Claude 先 Bash 修改文件，再用 `knowledge_replace` 修改同一路径，不丢失 proposal。
- 不同段落得到 rebased proposal；相同段落得到 conflicted。
- CRLF、UTF-8、相邻 hunks、create/delete 和大文件均有测试。

## 4. P0：Proposal claim、审批和崩溃恢复

### 4.1 状态机

```text
pending
  ├─ rebasing(claim_id) -> pending / conflicted / resolved_external
  └─ applying(claim_id) -> approved / failed
```

### 4.2 实现内容

- rebase 和 apply 均通过 compare-and-update 原子取得 claim。
- 所有状态更新验证 `claim_id`、claim kind 和 affected rows。
- apply 前记录 expected old/new hash，以及足以恢复的 bounded apply journal。
- 文件写入成功但数据库提交失败时，只允许 claim owner 执行补提交流程或安全回滚。
- 应用启动时恢复遗留 `applying`、`rebasing` 和 running ChatTurn/Activity。
- Approve、turn-end reconciliation、启动恢复和重复点击之间保持互斥且幂等。

### 4.3 验收

- 分别在 claim 后、写盘后、DB commit 前模拟崩溃，重启后都能得到确定的文件与 proposal 状态。
- 两个并发 Approve 只有一个 claim owner 能写盘。
- reconciliation 不会改写正在 applying 的 proposal。

## 5. P0：完整 Claude connection probe

### 5.1 实现内容

- 保留当前真实 Claude 两 turn、同 session resume 结构。
- turn 1 必须通过 Nest MCP create/list/read/replace，并通过正常 Knowledge Review/Apply 路径落盘。
- 等待索引完成后再开始 turn 2。
- turn 2 必须通过 search/read 验证 challenge 和 replace 后内容，再通过 Nest MCP delete，并正常 Apply。
- 验证六工具调用顺序、成功状态、返回 path/content/token，而不只检查 activity label。
- 验证 create/replace 后 search 可见，delete 后不可见。
- 任一 `finish_staged`、Apply、index wait 或响应语义失败都使 probe 失败。
- probe 全程取得 app-wide operation slot并支持取消。
- finally 路径清理临时 Pack、文件、session、turn、activities、proposals、index 数据、MCP credential/config 和 Claude probe session；任何残余使测试失败。

### 5.2 验收

- 故意让任一 MCP 工具返回错误内容、打乱顺序、跳过 Apply 或不更新索引，probe 均失败。
- 成功、CLI 错误、Stop、timeout 和 cleanup failure 均有集成测试。
- probe 完成后不存在 bearer credential 临时文件或测试 Pack 数据。

## 6. P1：Backend Descriptor 与 effective view

### 6.1 Backend registry

- 使用稳定字符串 `BackendId` 取代跨层封闭 `Nest | Claude` enum。
- 实现 Backend registry 和 `chat_backend_descriptors()`。
- Descriptor 包含 availability/reason、每 Mode availability、Model options、native tool profile、Knowledge profile 和 Settings target。
- composer 的 Backend/Model/Mode capsules 只消费 descriptors，不写死 Claude 分支。
- 未知历史 Backend 保留为 unavailable/read-only，不能导致 session 反序列化失败。
- `chat_update_selection` 和 `chat_send` 使用 descriptor 做 Backend、Model、Mode 校验。

### 6.2 Knowledge effective view

- search/list/read 使用一致的 effective view：turn-local staged 优先于 pending proposal，pending 优先于 disk/index。
- pending/staged create 和 replace 可被 search 命中。
- pending/staged delete 不再从 search 返回旧索引结果。
- conflicted、resolved_external 和 rejected 不形成 overlay。

## 7. P1：app-wide operation slot 与生命周期

- 将现有 chat-only `AtomicBool` 提升为带 operation kind/owner 的 app-wide slot。
- ChatTurn、connection probe、Save and connect、Test connection、Reindex 和 Vault 切换必须原子取得 slot。
- Settings 侧的检查不能只是瞬时 `ensure_no_chat_turn`；整个异步操作期间必须持有 guard。
- 删除正在生成的 session 时执行完整 Stop：撤销 lease、abort stage、终止 Claude 进程树、bounded reconciliation，再删除 session。
- 应用退出时执行有界清理并终结或在下次启动恢复遗留 running 状态。
- UI 根据同一 operation 状态禁用 composer、Claude Settings、Vault 控件和 Reindex，并显示当前 operation/跳转或 Stop 入口。

## 8. 实施顺序

1. 持久化 manifest、等待式 Reindex 和 degraded-mode capability gate。
2. turn-local staged/native 二次归并及大文件安全边界。
3. Proposal claim、apply journal 与启动恢复。
4. 两轮六工具完整 Claude probe。
5. Backend Descriptor、effective-view search 和 app-wide operation slot。
6. 完整自动化回归、Windows 手工验收、用户文档和 clean branch 收口。

前四项决定文件、Proposal 和索引一致性，应在架构扩展前完成。第五项是后续继续接入第三方成熟 Agent 的基础，不应从 Step 2 完成定义中移除。

## 9. 最终完成门槛

Step 2 完成时必须同时满足：

- Claude 连续会话可按每轮 Model/Mode 使用原生工具及全部 Nest Knowledge Capabilities。
- Sources、Tool Activities 和 Proposal 语义不会互相伪装。
- 成功、失败、Stop、冲突、Reindex 和崩溃后，文件、索引、proposal、turn 状态可恢复一致。
- Claude 原生修改不会被 proposal 审批意外覆盖。
- connection probe 能证明真实 Claude、MCP、Review/Apply、索引及清理全链路。
- composer 和数据库不再硬编码具体 Agent，新增 Backend 可以通过 descriptor/adapter 接入。
- Desktop UI 与 Rust 全量 sanity checks 通过，Windows 手工验收全部通过。
