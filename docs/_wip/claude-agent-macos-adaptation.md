# Claude Agent macOS 适配清单

> 状态：Step 2 后续平台工作
>
> 当前支持边界：Claude Agent Step 2 仅在原生 Windows 开发和验收；此文档不代表 macOS 已支持
>
> 上游策略：临时 WIP，不进入 `feat/claude-agent-clean`

## 1. CLI 发现与启动

- 把 `claude_cli.rs` 的候选发现拆成平台策略，保留共享的 detection、version probe、stream-json 和错误分类。
- macOS 接受无扩展名的原生 `claude` 可执行文件和 npm `cli-wrapper.cjs`；不能要求 `claude.exe`、`node.exe`、`.cmd` 或 `.ps1`。
- 自动发现至少检查进程 `PATH`、用户手工路径和常见用户级 bin/npm prefix。Finder 启动的 `.app` 不保证继承交互 shell 的 `PATH`，所以必须用真实 GUI 启动场景测试，不能只在 Terminal 中验证。
- `node` 查找改为平台文件名，并继续以参数数组启动 wrapper，不通过 shell 拼接命令。
- Settings 的 placeholder、检测结果和错误文案按平台生成；macOS 不显示 Windows 文件名提示。
- 为带空格、Unicode、符号链接和 Home 目录展开的 CLI/Vault 路径增加 macOS fixture。

## 2. 进程树、Stop 与退出

- 当前非 Windows `kill_tree` 只 `start_kill` 直接子进程，不能保证终止 Claude 派生的 shell/工具进程。macOS 启动时应建立独立 POSIX process group/session。
- Stop、timeout、删除 active session 和应用退出先向整个 process group 发送有界的温和终止，再升级为强制终止，并等待子进程回收。
- 验证 Node wrapper、Claude native binary、Bash 子进程和后台孙进程四种拓扑；退出后不得残留 MCP bearer、turn lease 或运行中 activity。
- 固化 macOS 的终止退出码/信号映射，避免把用户 Stop 分类为连接失败或把可恢复 session 标记为不可恢复。

## 3. 文件系统与 Vault 一致性

- 在 APFS 默认大小写不敏感以及大小写敏感卷上分别测试路径规范化、重复路径、manifest key 和 proposal claim。
- 验证 symlink、Unicode normalization、CRLF/LF、原子 rename 和临时文件替换；审批和恢复不能越出 Vault 或错误识别同一路径。
- 运行全部 effective-view、三方归并、apply journal、启动恢复和 Vault reconciliation 测试；再用 Claude 原生 Bash/Edit/Write 对 active Pack 做 create/replace/delete 实测。
- 验证 macOS 文件事件不是正确性的前提。Step 2 仍以 turn-end/startup/显式 Reindex reconciliation 为权威。

## 4. 应用运行环境与网络

- 在开发启动、直接打开 `.app`、DMG 安装后三种环境验证 Claude 登录凭据和用户 Claude 配置可被当前 OS 用户继承；Nest 不读取或复制凭据文件。
- 验证 loopback MCP 在 macOS 防火墙和系统代理配置下仍只绑定 `127.0.0.1`、校验 Origin 并要求 bearer credential。
- 核对 hardened runtime、签名、notarization 和应用权限对 CLI/Node/用户 shell 子进程的影响。当前 unsigned DMG 只能用于开发验证，不能替代发布验收。
- Apple Silicon 和 Intel 分别验证 native CLI、Node 架构以及 Rosetta 混用；错误信息必须能区分“找不到程序”和“架构不可执行”。

## 5. 测试与 CI

- 把 resolver/process fixture 中的 Windows 假设拆为共享用例加平台用例；Windows 现有用例不得被删除。
- 在 macOS runner 运行 Desktop UI 全套检查以及 Rust `fmt`、`clippy`、`test`。
- 增加 macOS fake CLI 集成：新 session、resume、session-id-in-use 回退、model switch、六工具 probe、Stop process group、timeout、stderr 快速失败和 cleanup residue。
- 在 Apple Silicon 与 Intel 产物上执行真实 Claude CLI 两轮 connection probe，并覆盖 Ask/Agent、native Bash、Nest MCP、Proposal Apply、Sources 和 Reindex recovery。
- 复跑 Windows 第 15 节验收，证明平台抽象没有回归当前唯一受支持环境。

## 6. 完成门槛

- Auto-detect、手工路径、Save and connect 在 Finder 启动的应用中可用。
- Stop/timeout/delete/exit 不遗留 Claude、Node 或 shell 进程树。
- 六工具真实 probe、连续会话、三 capsule、effective workspace、Proposal/冲突、native change reconciliation 全部通过。
- Apple Silicon、Intel 和 Windows 的自动化与手工矩阵均有可追溯结果。
- 用户文档和 Settings 不再声明“仅 Windows”，发布说明明确最低 macOS/Claude CLI 版本和 unsigned/signed 安装行为。
