# R7 终端迁移当前结构

2026-10-14，R7.1–R7.3 完成当前机器自动验收，R7.4 尚未开始。步骤见 [方案](terminal-migration-plan.md)，结果见 [R7.1](results/2026-10-14-r7-1/report.md)、[R7.2](results/2026-10-14-r7-2/report.md) 和 [R7.3](results/2026-10-14-r7-3/report.md)。

features/terminal 提供纯类型、屏幕/选区、输入/鼠标/OSC、参数/候选、稳定程序规则及 TerminalModel/TerminalMetadata。模型只持元数据，10 个私有字段登记所有者；没有 controller、服务、PTY 或 profile 副本。domain/application_layout 提供无 GUI 的布局拓扑和计划，GUI 键适配在 ui/terminal/keys。

AppRuntime.terminals 是会话运行时所有者。infrastructure/terminal/runtime 通过 registry 按 SessionId 持有控制器；session 同步 parser/进程与线程，pty 管理平台启动和终止，profiles 发现 shell，integration 管理 cwd/脚本，program 管理原生探测和隐藏 observer。已关闭身份不能重建；shutdown 明确释放资源，不依赖视图引用计数。

app/terminal_binding 作为 R7.4–R7.6 前的临时接线：TerminalStore 底层是独立 State<TerminalModel>，TerminalTabs/TerminalTab 是投影；服务引用及会话句柄均为 Weak，短暂访问按 SessionId 查找。实时 cwd 和资源效果在 State 锁外取得/执行。关闭先更新模型/布局，再释放注册资源。Profile getter 来自当前 AppState；GUI 通知及 Palette 适配在 app/terminal_runtime。

纯布局、PTY quoting/环境、脚本、原生探测、会话 JSON、既有线程与轮询保持原样。旧 terminal_session、terminal_mouse、model/application_layout、terminal_profiles、terminal_directory、terminal_program 已删除，无兼容重导出。纯终端模型和基础设施无例外；legacy exception 为 35 个，根/editor/workspace 字段 40/31/9、根 UI 适配器 16。

完整状态/应用命令、通知/effect 世代与窄 UI 留待 R7.4–R7.6。当前仅完成 R7.3 本机自动验收，未宣称 R7 整体结束。本机 Rust 1.98.0；固定 1.97.1、原生人工、其他平台、远端 CI、安装打包和完整性能 A/B 仍待验。
