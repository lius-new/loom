# 整体重构进度

2026-10-13：阶段 0 的本机基线与检查修复、阶段 1 的 R1.1–R1.5、阶段 2 的 R2.1–R2.6 本机自动验收已完成。阶段 3 的独立核心、应用编辑事务、交互状态和文本 UI 窄接口已完成本机自动验收；阶段 4 工作区域与文件/会话适配已完成本机自动验收；阶段 5 Explorer 迁移（R5.1–R5.6）已完成本机自动验收；阶段 6 Git 迁移进行中（[方案](git-migration-plan.md)），R6.1–R6.3 已完成本机自动验收；阶段 7–8 尚未开始。

2026-10-13 阶段 3 前置历史记录：恢复 Skia 本地缓存构建，固定 Rust 1.97.1 并同步 CI，复跑本机自动验收与 31 项性能基线；release 与 packaged 应用隔离启动、portable 打包和安装器编译通过。当时阶段 3 尚未开始，详见 [前置报告](results/2026-10-13-pre-r3/report.md)。原生人工、跨平台、远端 CI 与真实安装/卸载验收仍待执行。

- [x] 阶段 0：检查修复与本机行为/测试/性能基线
- [x] 阶段 1：接线、状态归属、运行时边界、窄接口及门禁
- [x] R2.1 Search 请求/结果、匹配器、私有模型及注入源端口
- [x] R2.2 Search UI 输入、IME、展开、滚动和焦点状态
- [x] R2.3 应用任务协调、debounce、取消、generation 时序
- [x] R2.4 导航意图和应用层文档打开
- [x] R2.5 通用文档快照，移除 Workspace 的 Search DTO
- [x] R2.6 Search 取消根状态直接访问，Search 无架构例外
- [x] 当前机器 R3 前置：固定工具链、恢复构建、完整回归与本机性能基线
- [ ] 原生 IME/视觉/系统焦点、macOS/Linux、远端 CI、安装包验收
- [x] R3.1 独立编辑核心提取与无 GUI 测试入口
- [x] 阶段 3：应用编辑事务、交互状态与文本 UI 窄接口迁移（本机自动验收）
- [x] 阶段 4：Workspace/Document/Pane 域与文件/会话适配（本机自动验收）
- [x] 阶段 5：Explorer 迁移（本机自动验收，[说明](explorer-migration.md)、[方案](explorer-migration-plan.md)）
  - [x] R5.1 类型、端口、纯规则与文件系统适配（本机自动验收）
  - [x] R5.2 Explorer 模型/UI 状态与抽屉 chrome 归属（本机自动验收）
  - [x] R5.3 Explorer 命令与跨功能事务迁入 app/explorer（本机自动验收）
  - [x] R5.4 watcher 竞态测试与准备阶段性能对照（本机自动验收）
  - [x] R5.5 Explorer 视图与右键菜单改为投影 + 语义命令（本机自动验收）
  - [x] R5.6 删除根状态别名、关闭 R5 登记、门禁与文档
- [ ] 阶段 6：Git（[方案](git-migration-plan.md)）
  - [x] R6.1 纯 Git 规则迁入 features/git（本机自动验收）
  - [x] R6.2 Git 进程适配迁入 infrastructure/git，删除 src/git（本机自动验收）
  - [x] R6.3 Git 模型/UI 状态、左侧抽屉 chrome 与 diff 分栏归属（本机自动验收）
  - [ ] R6.4–R6.7
- [ ] 阶段 7–8

当前完整 workspace 测试 650 通过、0 失败、13 忽略（应用 566，编辑核心 68，独立域重复验证 16；忽略项含新增 prepare_batch 基准）；R4 清单全部保留，R5 新增 35 项。门禁 23 项自测通过，登记根字段 76、editor 30、workspace 核心 9、Explorer 19，legacy exception 文件 57。详见 [R5 报告](results/2026-10-13-r5-explorer/report.md)；31 项常规性能基线需 PowerShell 7，待补跑。

报告：[阶段 0](../baseline/README.md)、[阶段 1](results/2026-10-13-phase1/report.md)、[阶段 2](results/2026-10-13-phase2/report.md)。后续修改必须清理 policy 中过期的例外和字段登记，不得整体刷新登记绕过门禁。
