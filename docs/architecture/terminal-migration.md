# R7 终端迁移当前结构

2026-10-14：R7.1–R7.7 本机自动验收完成。见 [方案](terminal-migration-plan.md) 和 [最终验收](results/2026-10-14-r7-terminal/report.md)。R8 未开始。

features/terminal 提供纯类型、TerminalModel/TerminalMetadata、屏幕/选区、输入/鼠标/OSC、profile 参数和候选、稳定程序和菜单目标规则。模型只保存私有元数据，3 项模型字段和 7 项会话字段登记；不含控制器、PTY、线程、writer 或 profile 服务。domain/application_layout 管理纯布局拓扑和计划，GUI 键适配留在 ui/terminal/keys。

AppRuntime.terminals 是运行时所有者。infrastructure/terminal 的 registry/runtime 按 SessionId 管理控制器，session/pty/profiles/integration/program 管理原生进程、线程、shell 发现、目录与前台程序探测。关闭身份永久退休，shutdown/close 幂等。app/terminal_binding 将 State<TerminalModel> 与弱资源访问连接；保存模型、投影、会话句柄或 UI 端口不会保活进程。移动只改元数据和布局，保持同一 PTY；split 才新建资源。

AppState 组合 ApplicationLayoutState、TerminalChrome 和 TerminalPanelState。13 项旧根字段与 editor 终端点击状态已迁到明确所有者，22 项终端归属字段登记。UI 选择/焦点/菜单/滚动/表单/拖动属 ui/terminal/state，默认区域显示/高度属 app chrome，布局/sash 属 app layout。default_shell、terminal_profiles、shell_options、terminal_cursor_blink、terminal_font、terminal_font_input 六项配置归 R8，暂保留原设置所有者。

app/terminal 协调新建、选择、split、关闭/批量关闭、用途、toggle、输入/剪贴板/鼠标；按 SessionId 和菜单目标执行前重查。先更新模型与布局、完成回合检查归属，再处置原生资源，不宣称跨 store 原子事务。app/terminal_profiles 协调 profile 校验、ID、发现、保存/删除/default 与项目 shell 提示；关闭后保留草稿给迟到 blur。

app/terminal_effects 协调恢复、profile 刷新、可见区域启动、探测、label、blink、保存和清理。reader/waiter 在生命周期锁内核对 generation 并提交，通知在锁外调用；UI 队列按 SessionId、generation 和 effect lease 拒收旧结果。mount 代次防止旧 cleanup 停止新 mount。隐藏区域不杀进程，保存执行时读实时 cwd，8 项会话 JSON 字段和未保存文档退出顺序保持。

ui/terminal/{view,menu,settings,layout} 仅接收值帧、TerminalUiState 投影和语义命令端口，保留绘制、键鼠适配和框架 capture/hover/release 事实。app/terminal_view 组合端口，批量读取资源元数据后生成纯帧；资源查找锁不跨控制器检查。app/layout_map 持有 MapStore、几何场景、计划提交和 settle，保留独立订阅、区域内零额外通知。UI 不访问 AppState、Workspace、AppRuntime、控制器或基础设施。

旧 terminal_session、terminal_mouse、terminal_directory、terminal_profiles、terminal_program、model/application_layout，以及四个旧 UI 路径已删除，无别名或兼容重导出。终端 feature/infrastructure/UI 无例外、无根 UI 适配器；全仓库仍有 31 项 R8 遗留例外和 12 个其他领域根 UI 适配器，根/editor/workspace 字段为 30/30/9。

最终 workspace 三种环境均 711 通过、15 ignored；原 603 应用入口和原 14 ignored 无丢失，新增 25 项行为、13 项域重复验证和 1 项 opt-in 基准分列。工程门禁、四项原生 smoke、隔离启动/恢复通过。性能和剩余人工/跨平台验收详见最终报告；实际 Rust 1.98.0，固定 1.97.1 尚未复验，不能据此宣称全部平台验收完成。
