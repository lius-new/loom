# R7 终端迁移当前结构

2026-10-14，R7.1–R7.6 完成当前机器自动验收，R7.7 尚未开始。步骤见 [方案](terminal-migration-plan.md)，当前结果见 [R7.4 验收](results/2026-10-14-r7-4/report.md)。

features/terminal 提供纯类型、屏幕/选区、输入/鼠标/OSC、参数/候选、稳定程序规则、TerminalModel/TerminalMetadata、TerminalAvailability 与菜单 close_targets 规则。会话模型只持元数据，10 私有字段登记；没有 controller、服务、PTY 或 profile 副本。domain/application_layout 提供纯布局拓扑和计划，GUI 键适配在 ui/terminal/keys。

AppRuntime.terminals 是会话运行时所有者，infrastructure/terminal/runtime/registry 按 SessionId 持有控制器；session/pty/profiles/integration/program 负责原生资源、shell、目录与进程探测。关闭身份不能重建，shutdown 明确释放资源。app/terminal_binding 仍是 effect 和完整窄视图迁移前的临时绑定：底层 State<TerminalModel>，投影和会话句柄通过 Weak 按身份访问资源，快照不保活进程。

AppState 组合 ApplicationLayoutState、TerminalChrome 和 TerminalPanelState（feature availability + TerminalUiState）。13 项旧根字段和 editor 终端点击状态已迁出；交互/草稿属 ui/terminal/state，默认区域显示/高度属 app chrome，布局/sash 属 app layout。六项设置字段仍属 R8。

app/terminal 协调新建、选择、split、关闭/批量关闭、用途、toggle、输入、剪贴板与鼠标操作。命令持明确 SessionId/菜单目标，执行前重查目标；模型和布局顺序更新，完成回合检查归属，随后清理原生资源。app/terminal_profiles 处理校验/ID/发现/保存/删除/default 和项目 shell 提示；ui/terminal_settings 仅接收 Frame、Command、StateAccess，输入使用 projected binding。草稿在关闭后保留给迟到 blur。

旧 terminal_session、terminal_mouse、model/application_layout、terminal_profiles、terminal_directory、terminal_program 已删除。pure feature 和 infrastructure 无例外；legacy exception 34，根/editor/workspace 字段 30/30/9，根 UI 适配器 15，22 项新终端归属字段登记。

R7.5 effect/通知/持久化协调和 R7.6 完整终端/布局窄视图尚未开始；shell 内生产 effect 仅改状态访问路径。当前实际 Rust 1.98.0；固定 1.97.1、原生人工、其他平台、远端 CI、安装打包及完整性能 A/B 保留待验，R7 整体尚未结束。

R7.5 本机自动验收：effect/恢复/启动/探测/保存/清理迁入 app/terminal_effects，输出/退出生命周期锁内核对代次，UI 通知按 SessionId/代次/effect lease 拒收旧结果，旧 cleanup 不停止替换 mount。710 通过、14 忽略，新增 8 项，619 旧应用入口无缺失；47 自测、架构、fmt、Clippy、release、四原生 smoke、隔离启动通过。实际 Rust 1.98.0；固定工具链、原生人工/跨平台和最终性能验收保留。详见 [R7.5 验收](results/2026-10-14-r7-5/report.md)。继续 R7.6，R8 未开始。

R7.6：四旧 UI 路径删除，新视图只接收帧、投影和命令。app 持有 MapStore、几何计划和 settle，UI 保留 hover/release。711 通过、14 忽略，627 个旧应用入口无缺失，新增 1 项端口不保活运行时回归。48 自测、门禁、fmt、Clippy、release、四原生 smoke、隔离启动通过。238 Rust 文件、31 例外、12 根 UI 适配器。详见 [R7.6 验收](results/2026-10-14-r7-6/report.md)。继续 R7.7，R8 未开始。
