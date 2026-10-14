# Loom R7.3 迁移验收

2026-10-14，分支 `refactor/r7-terminal`，基础提交 `ebf18ff`。终端会话模型与运行时所有权分离已完成当前机器自动验收。停在 R7.3，R7.4 未开始。

## 结果与所有权

`features/terminal/model` 的 TerminalModel 只保存会话元数据，继续位于独立的 State，未并入 AppState。模型的 tabs、active_id、next_id 三个字段和会话元数据的七个字段均私有并逐项登记。模型不保存 controller、PTY、线程、writer、服务引用或第二份 profile 配置。

`AppRuntime.terminals` 持有运行时服务；`infrastructure/terminal/runtime` 按 SessionId 管理资源，registry 是生产运行时的所有者。新建/split 分配独立资源，移动、分组和区域投影仅修改元数据与布局；PTY 仍按原有可见会话 effect 同步启动。保留 u64 身份分配、恢复及相邻关闭规则。

旧 UI 暂时通过 `app/terminal_binding` 的 TerminalStore 接线。底层 State 是纯 TerminalModel，模型/区域投影与 TerminalHandle 仅持有 Weak 服务引用，操作时短暂获取对应资源；UI 不再克隆强控制器。旧 `terminal_session.rs` 已删除，没有兼容重导出。窄视图和完整语义命令仍按 R7.4–R7.6 推进，本轮没有提前实施。

运行时 cwd 在 State 锁外取得，split 和保存仍使用即时目录。资源创建在模型更新完成后进行；关闭先移除模型和布局引用，再在锁外移除注册资源并终止。已关闭身份不可从旧快照重新登记，旧句柄不再执行进程操作；重复 close/shutdown 不重复释放。启动与最终 dispose 通过生命周期锁协调，启动通知在释放该锁之后投递，允许通知中触发自身清理。应用 effect 清理和 AppRuntime Drop 均显式 shutdown，不依赖模型/视图快照的 Arc 数量或 GUI 回调完成释放。

Profile 配置通过 app 注入的 getter 读取当前 AppState，模型没有配置副本。运行时仅保存每个会话启动所需的配置；profile 更新保留原 State 通知语义。Pty 启动、quoting、环境合并、目录脚本、原生探测、observer、布局纯算法及既有线程/轮询频率保持原样。

## 验收

| 检查 | 结果 |
| --- | --- |
| 修改前完整 workspace | 689 通过、0 失败、14 忽略 |
| 修改后完整 workspace | 696 通过、0 失败、14 忽略 |
| 应用 / 编辑核心 / 独立域重复验证 | 599 / 68 / 29 通过 |
| 旧应用执行入口映射 | 606 项全部找到对应结果 |
| 原测试迁移 | 19 项：5 项纯模型、14 项 app 原生/跨层回归；无缺失或 ignored 变化 |
| 新增行为覆盖 | 7 项，包含 fake 资源计数、弱快照、关闭、移动/split、真实退出、通知重入和配置事实源 |
| 架构及报告自测 | 42 通过：架构 39、报告 3 |
| 架构门禁 | 221 个 Rust 源文件；纯终端模型与基础设施无例外 |
| 全目标编译、fmt、严格 CI Clippy | 通过，无新增 lint 豁免 |
| release 构建与隔离启动 | 通过，正常关闭并保存 synthetic 会话，无强制清理 |
| 四项 opt-in Windows 原生 smoke | 最终测试二进制按精确入口单项运行，全部通过 |
| 原算法源码对照 | 36 项通过，所有权/边界替换单列 |

696 = 应用 599 + 编辑核心 68 + 独立域重复验证 29。独立域数量保持 29，没有把重复验证算作本轮新增行为。

Fake registry 按 SessionId 记录创建/释放：移动不创建或释放，split 仅新增一个，关闭和 shutdown 各释放一次，资源 stop 在 registry 锁外执行。持有模型、区域及操作句柄快照时，应用运行时仍能释放服务和进程。

真实 ConPTY 测试提交实际布局移动计划，验证 PID 与 generation 不变；保留快照并释放应用运行时后，通过预先取得的 Windows 进程句柄确认本次创建的 shell 与 observer 都已退出。旧弱句柄及保留的测试检查句柄均不能重新启动已 dispose 的资源。另验证启动通知中关闭自身没有死锁。原有移动 leader 的 cwd、scrollback、选区、resize、分组及相邻关闭回归继续通过。

四项 smoke 和 `native_mouse_probe` 的入口同步到 `app::terminal_session_tests::tests::`；baseline 脚本的精确筛选与辅助进程命令已同步。未全局执行所有 ignored 测试。

门禁拒绝纯模型的 GUI/I/O、运行时回嵌及类型别名、未登记/公开字段；拒绝将 State 或投影改成强资源所有者、通过父路径/源码包含绕过，以及恢复旧顶层模块。遗留例外从 36 减至 35；根/editor/workspace 字段仍为 40/31/9，根 UI 适配器仍为 16。

## 记录与边界

检查记录、测试映射、算法对照及源文件 SHA-256 保存在 `docs/architecture/results/2026-10-14-r7-3`；修改前源码/二进制、最终测试和 release 二进制、原始日志归档到 `target/baseline/2026-10-14-r7-3`。隔离启动只操作本次创建的 PID，使用独立 APPDATA/LOCALAPPDATA，不改真实会话。

本机实际 Rust 1.98.0；项目固定 1.97.1、原生人工视觉/IME/系统焦点、Linux/macOS、远端 CI 和安装打包仍待验证。Cargo.toml、Cargo.lock、工具链配置、build.rs 和 observer 源码构建链保持不变；program 模块只新增 cfg(test) PID 检查接口。本轮没有完整性能 A/B，沿用 R7.1 的同机 before 基线，不宣称性能提升。

R7.4 的状态与应用命令、R7.5 的完整通知/effect 世代协调、R7.6 的窄视图及最终收尾均未推进。

最终 release SHA-256：`797B04FA0DCE8F2A7ADB8C1F015004522188BE854639F440978A44BBCFD98869`。隔离启动 PID：13264，退出码 0；observer 构建产物与实际提取文件 SHA-256 一致。
