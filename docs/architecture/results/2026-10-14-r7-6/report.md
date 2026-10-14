# R7.6 窄终端视图与布局缩略图验收

2026-10-14，基于 `f7b06d99d948a69717ebbca0be1acb3038c31184`，分支 `refactor/r7-terminal`。R7.6 本机自动验收完成，继续 R7.7。

四个旧 UI 路径（terminal.rs、terminal_settings.rs、terminal_tab_context_menu.rs、layout_map.rs）已删除，ui/mod 的旧声明同步移除，没有兼容重导出。新视图位于 ui/terminal/{view,menu,settings,layout}，只接收只读会话/屏幕帧、TerminalUiState 投影、焦点/帧请求能力以及语义命令端口，不接收根状态、controller、AppRuntime 或 infrastructure。

app/terminal_view 构造帧并协调 SessionId 命令。app/layout_map 持有独立瞬态 MapStore、通知、MapScene 准备、计划提交和 settle；UI 保留绘制、框架 pointer capture 与 release/hover 事实。布局读取端口借用只读 drag 值，保留 zone 内零通知和独立订阅机制。模型和布局锁不嵌套，SessionId 列表在根状态锁外取得。

完整 workspace **711 通过、0 失败、14 ignored**（应用 614、编辑核心 68、独立域重复验证 29）。627 个旧应用入口无缺失，20 个旧 UI/布局测试保留原名迁入 app，新增 1 项端口/帧保留时不保活运行时的回归。原有 UiSession 测试继续覆盖 inactive split 输入、modal 菜单、非活动目标、关闭焦点、profile blur/unmount、5 px threshold、cancel PointerUp、cached preview、T junction、窗口收缩、无效落点与 zone 内零通知。

48 Node 自测（架构 45、报告 3）、严格架构门禁、all-targets、fmt、CI Clippy、release、四精确 native smoke 和隔离启动通过。238 Rust 源文件，根/editor/workspace 30/30/9，遗留例外由 34 降至 31，根 UI 适配器由 15 降至 12。纯规则、PTY、原生探测、域布局和 JSON schema 源码保留。release SHA-256：`2BE54C531D6E40D6780E3D44F6002056489FD7755965437038B14AFA61B071F4`；隔离启动 PID 21108 正常退出 0，无强制清理。

原始记录在 `target/baseline/2026-10-14-r7-6`。实际 Rust/Cargo 1.98.0，固定 1.97.1、原生人工/IME/系统焦点、Linux/macOS、远端 CI/安装及最终性能 A/B 保留待验。本检查点 R7.7 和 R8 尚未完成。
