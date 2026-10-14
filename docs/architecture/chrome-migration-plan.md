# R8 设置 快捷键与应用 chrome 收尾方案

2026-10-14，起点 0caaf0701f35a0b437523f0b06fa0b8550e8e9b9，工作目录 C:\Users\yys\loom，执行分支 refactor/r8-chrome。R7 已合并到本地 main。R8 开始前 711 通过、15 ignored；源码和 debug/release 测试二进制冻结在 target/baseline/2026-10-14-r8-base。

本阶段处理现有 31 项 R8 例外、12 个根 UI 适配器和剩余设置/keymap/chrome 状态；保留现有交互、JSONC 注释及文件格式、单次 workspace 事务、PTY 所有权和退出保存流程。不得刷新例外或整体重写字段登记掩盖边界。

## 当前状态

R8.1–R8.6 已完成本机自动验收，完整结果见 [最终验收](results/2026-10-14-r8-chrome/report.md)。以下逐步执行记录保留当时状态；固定工具链、人工/跨平台/远端 CI 验收仍待执行。

## 检查点

1. R8.1：设置/字体/主题 ID/JSONC 纯类型和规则；设置与项目配置文件、配置 watcher 迁入基础设施；绘制资源归 UI。
2. R8.2：设置私有模型、设置页输入状态及命令；快捷键纯规则、键事件适配、上下文事实投影和序列状态分离。
3. R8.3：文件打开/保存/关闭与会话协调归 app，文件加载/监听原生工作归 infrastructure；持久化兼容、过期结果与退出清理不退化。
4. R8.4：titlebar/statusbar/welcome/home/tabs/菜单/关闭确认/窗格与 diff chrome 改为值帧、局部 UI 状态和语义命令；不将绘制文件整体移进 app。
5. R8.5：明确 chrome 状态归属，移除 AppState 弱 runtime 能力桥接和旧根别名；保留单次 workspace 事务，不承诺未经测量的根克隆改善。
6. R8.6：清理 R8 例外/适配器登记，强化负例，逐项映射原测试；最终默认/独立 Git/release 全量回归、工程门禁、隔离启动/恢复和同机性能复验。

## 每步验收

- [x] 原测试路径、ignored 和执行结果映射无缺失；新增测试只验证必要的边界/竞态。
- [x] 架构门禁和负例自测、fmt、all-targets、严格 CI Clippy、完整 workspace 测试。
- [x] release 构建与隔离启动；影响 PTY/线程清理时加跑原生专项。
- [x] 设置/项目 JSONC、快捷键覆盖/上下文/序列、保存/关闭/恢复行为兼容。
- [x] UI 只读帧不携带 AppState、AppRuntime、原生服务或可变 workspace；资源留在运行时。
- [x] 冻结 before/after release 二进制、同机相同工具链，预热与正式交错性能样本，调查持续超过 10% 的上升。
- [x] 最终工作区干净、本地独立检查点和完整证据；不自动推送。

实际本机 Rust 1.98.0；固定 1.97.1、原生人工 IME/视觉/焦点、Linux/macOS、远端 CI 和真实安装/卸载仍需单列待验，不能扩大本机自动验收结论。

## R8.1 执行记录

2026-10-14：设置/字体/主题 ID/JSONC 纯规则、配置文件/watcher 平台适配与 UI 绘制资源拆分。711 通过、15 ignored，629 个旧应用入口无缺失，51 项源码测试迁移；51 自测、架构、fmt、Clippy、release 和隔离启动通过。245 Rust 文件，遗留例外 31→23；根字段/根 UI 接口仍归后续检查点。详见 [验收](results/2026-10-14-r8-1/report.md)。继续 R8.2，R8 整体未完成。

## R8.2 执行记录

2026-10-14：设置/keymap 私有纯模型、UI 输入投影和值帧/命令、键上下文事实和 GUI 解码分离；keymap 文件 I/O 及配置 own-write 读取归基础设施。713 通过、15 ignored，629 个旧应用入口无缺失，新增 2 项；53 自测、门禁、fmt、严格 Clippy、release 和隔离启动通过。254 Rust 文件、19 根字段、15 例外、10 根 UI 适配器。详见 [验收](results/2026-10-14-r8-2/report.md)。继续 R8.3，R8 整体未完成。

## R8.3 执行记录

2026-10-14：文件 work/timer、原生 watcher/批次、读写/对话框、窗口探测与启动参数归基础设施；app 协调文件请求、跨模型对账、会话保存与关闭。弱 lease 拒收服务关闭后已排队交付，新增 2 项真实资源/队列回归。715 通过、15 ignored，631 旧应用入口无缺失；54 自测、门禁、fmt、Clippy、release 和隔离启动通过。260 Rust 文件、19 根字段、12 例外、10 根 UI 适配器。详见 [验收](results/2026-10-14-r8-3/report.md)。继续 R8.4，R8 整体未完成。

## R8.4 执行记录

2026-10-14：chrome 全部经值帧/语义命令；tabs 和 diff 协调归 app。715 通过、15 ignored，旧应用入口无缺失；56 自测、架构、fmt、严格 Clippy、release 与隔离启动通过，根 UI 适配器降至 0、遗留例外 2。详见 [验收](results/2026-10-14-r8-4/report.md)。继续 R8.5。

## R8.5 执行记录

2026-10-14：模型移除弱 runtime，服务由显式 AppContext 注入；chrome owner 与旧根别名清理完成。715 通过、15 ignored；59 自测、静态门禁、release、隔离启动与 4 原生 ConPTY 回归通过。根字段 15、例外 0、根 UI 适配器 0。详见 [验收](results/2026-10-14-r8-5/report.md)。继续 R8.6。

## R8.6 执行记录

2026-10-14：默认/独立 Git/release 三轮均 715 通过、15 ignored；629 原应用入口与 698 原声明无缺失。59 自测、静态门禁、4 原生 ConPTY、隔离启动与 8 字段恢复通过。43 指标完成冻结二进制同机交错 A/B；首轮 Search 取消上升已保留并经 30 对复测调查，未持续复现。R8 本机自动验收完成，限制见 [最终报告](results/2026-10-14-r8-chrome/report.md)。
