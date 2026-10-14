# R7 终端迁移当前结构

2026-10-14，R7.1 和 R7.2 完成当前机器自动验收，R7.3 尚未开始。步骤见 [方案](terminal-migration-plan.md)，验收见 [R7.1](results/2026-10-14-r7-1/report.md) 和 [R7.2](results/2026-10-14-r7-2/report.md)。

features/terminal 提供纯终端类型、屏幕/run/选区、输入/滚轮/OSC、参数/候选和稳定程序规则；domain/application_layout 提供无 GUI 的布局拓扑和计划。GUI 键事件适配位于 ui/terminal/keys。

infrastructure/terminal 集中 PTY/进程资源：session 维护控制器与线程，pty 负责平台启动和清理，profiles 发现可执行 shell，integration 管理目录和临时启动脚本，program 查询进程与隐藏 console observer。notification 只持有应用注入的同步回调。app/terminal_runtime 适配 GUI 重绘与 theme Palette，基础设施无 GUI 或 app 反向依赖。

terminal_session 保留 TerminalTab/TerminalTabs；模型和区域快照仍共享控制器并延长资源生命周期。R7.3 才分离纯模型和按 SessionId 管理的运行时。effect、应用命令、根状态、配置卡片和 UI 接线仍留待 R7.4–R7.6；本轮仅更新所需路径和边界调用。

旧 terminal_mouse、model/application_layout、terminal_profiles、terminal_directory、terminal_program 路径已删除，没有兼容重导出。终端纯规则、布局域和终端基础设施均无迁移例外；剩余 legacy exception 为 36 个。根/editor/workspace 字段 40/31/9，根 UI 适配器 16，尚未宣称 R7 整体收尾。

保留同步启动、原线程和轮询结构、参数引用、环境合并、脚本生命周期、失败文案、依赖与会话 JSON。本机 Rust 1.98.0；固定 1.97.1、原生人工、其他平台、远端 CI、安装打包和完整性能 A/B 仍待验。
