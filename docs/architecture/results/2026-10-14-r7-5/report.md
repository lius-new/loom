# R7.5 effect、通知与会话协调验收

2026-10-14，基于 `6cb8d0d7cce90f81c9b46a1082a74e101fa3c405`，分支 `refactor/r7-terminal`。R7.5 本机自动验收完成，继续推进 R7.6。

恢复、profile 接线/更新、程序/cwd 探测、可见区域启动/resize、保存、owner 清理和光标 blink 协调已进入 app/terminal_effects；shell 保留安装入口。250 ms 探测、1 秒标签、500 ms blink、8 KiB reader、2000 行历史和现有原生启动保持。无会话时不扫描系统进程，隐藏默认区域不会启动/resize/终止其会话。

探测结果带 SessionId 与运行代次，UI 投递重查存活身份、代次和独立 effect lease。旧 effect 停止自己的 worker 并作废自己的 queued 通知，旧 mount 清理不能停止替换 mount。reader 的输出/cwd/失败和 waiter 的退出检查与状态写入共享 lifecycle 锁；通知回调在锁外执行，并按弱 SessionId 句柄核对代次。纯模型不接收屏幕输出，不保活原生资源。

profile getter 在恢复会话的第一份投影前接入，保持初始名称和命令参数。保存实际执行时重新读取实时 cwd；正常退出与含脏文件退出中的原有保存顺序保留。八个终端 JSON 字段、ShellKind/Custom(u64)、分组/宽度/用途/cwd 恢复和缺字段/空会话兼容，未持久化程序标签或布局拓扑。

完整 workspace **710 通过、0 失败、14 ignored**（应用 613、编辑核心 68、独立域重复验证 29）；新增 8 项行为测试，619 个旧应用入口无缺失、ignored 未变。确定性 UiTasks/ManualClock 验证旧 run、close、旧 effect、迟到 cwd/程序通知、249/250 ms 与 999/1000 ms、restart 标签重置、mount cleanup、隐藏区域、旧 JSON 和 profile 首帧。原生输出/cwd/失败/退出晚到通过实际生产更新入口的代次测试验证。

47 项 Node 自测（架构 44、报告 3）、架构门禁、all-targets、fmt、严格 CI Clippy、release、四项精确原生 smoke、隔离启动全部通过。232 Rust 源文件，根/editor/workspace 30/30/9，34 遗留例外；13 项纯规则/PTY/原生探测/JSON/schema 源码对照一致。release SHA-256：`70A8152EB167F331F7A0CFE917FF1ED11FAF25270B7C7411790F4E011EA1ED0B`，隔离启动 PID 10608 正常退出 0，无强制清理。原始日志和二进制在 `target/baseline/2026-10-14-r7-5`。

本机实际 Rust/Cargo 1.98.0；固定 1.97.1、原生人工视觉/IME/系统焦点、Linux/macOS、远端 CI/安装及 R7 最终性能 A/B 保留待验。R7.6/R7.7 和 R8 在此检查点未完成。
