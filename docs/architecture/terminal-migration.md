# R7.1 终端纯规则与应用布局域

2026-10-14，R7.1 当前机器自动验收完成，R7.2 尚未开始。实施步骤见 [方案](terminal-migration-plan.md)，检查记录见 [验收](results/2026-10-14-r7-1/report.md)。

纯终端数据与规则在 features/terminal：types 提供 shell、身份及展示 DTO，screen 提供选区/run/调色板转换，input/mouse 处理无 GUI 协议，directory 解析 OSC，profiles 提供参数和给定候选的选择，program 处理给定进程信息和稳定标签。UI 的键转换在 ui/terminal/keys。

屏幕 Palette 由现有运行时读取 theme 后传入，feature 无 theme、GUI、根状态或 I/O；输入修饰键在 UI 转换。application_layout 使用纯 Rect/Axis/Direction，app/UI 转换到 UiRect。布局数学与原测试保持原样，workspace-tests 编译同一份域源码。

当前仍由 terminal_session 持有 controller/PTY/线程，terminal_profiles 保留发现及自定义 option 的系统解析，terminal_directory 保留启动集成与目录验证，terminal_program 保留 OS probe/observer。TerminalTabs、effect、根状态和 UI 仍按后续阶段登记，不声称会话快照已脱离运行时。

旧 terminal_mouse 和 model/application_layout 已删除，无 re-export；门禁覆盖纯模块、旧路径和旧模块类型别名。状态字段归属保持 R7.1 前的 40/31/9 项，迁移例外为 39 个。实际使用 Rust 1.98.0，项目仍固定 1.97.1；原生人工、其他平台、远端 CI 与固定版本验证保留待验。
