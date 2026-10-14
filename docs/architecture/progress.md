# 整体重构进度

2026-10-14：R7.1 终端纯类型/规则和应用布局域迁移已完成当前机器自动验收（[说明](terminal-migration.md)、[方案](terminal-migration-plan.md)、[验收](results/2026-10-14-r7-1/report.md)）；R7.2–R7.7 和阶段 8 尚未开始。本机 Rust 1.98.0，固定 1.97.1 验证仍待执行。

2026-10-13：阶段 0–5 本机自动验收已完成；阶段 6 Git 迁移（R6.1–R6.7）已完成本机自动验收（[说明](git-migration.md)、[方案](git-migration-plan.md)、[验收](results/2026-10-13-r6-git/report.md)）；阶段 7–8 尚未开始。

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
- [x] 阶段 6：Git（本机自动验收，[方案](git-migration-plan.md)）
  - [x] R6.1 纯 Git 规则迁入 features/git（本机自动验收）
  - [x] R6.2 Git 进程适配迁入 infrastructure/git，删除 src/git（本机自动验收）
  - [x] R6.3 Git 模型/UI 状态、左侧抽屉 chrome 与 diff 分栏归属（本机自动验收）
  - [x] R6.4 Git 命令迁入 app/git，写操作经可注入执行器（本机自动验收）
  - [x] R6.5 Git 轮询与扫描发布迁入 app/git_poll，修复固定仓库时扫描被丢弃（本机自动验收）
  - [x] R6.6 Git 视图投影、只读快照与语义命令，hunk/克隆接线迁出 UI
  - [x] R6.7 门禁、例外清理、测试映射、性能对照与文档
- [ ] 阶段 7：Terminal
  - [x] R7.1 纯类型/屏幕/输入/OSC/参数/候选规则和应用布局域（当前机器自动验收）
  - [ ] R7.2 PTY 与系统适配
  - [ ] R7.3 会话模型和运行时所有权
  - [ ] R7.4 状态归属与应用命令
  - [ ] R7.5 effect、通知与持久化
  - [ ] R7.6 窄视图与布局缩略图
  - [ ] R7.7 门禁与完整收尾
- [ ] 阶段 8

R6 截止完整 workspace 测试 673 通过、0 失败、14 忽略（应用 589，编辑核心 68，独立域重复验证 16；新增忽略项为 Git 性能基准）。R6 新增 23 项行为测试，本轮 30 项测试迁移无缺失。架构自测 29 项、报告自测 3 项通过；登记根字段 40、editor 31、workspace 核心 9、Explorer 19、Git 33，legacy exception 文件 41。31 项常规性能基线已用 PowerShell 7 补跑；Git 大仓库冻结二进制 A/B 与隔离启动/恢复完成。首次隔离回归出现一次现有 ConPTY scrollback 断言波动，最终完整复跑通过，失败记录保留。详见 [R6 报告](results/2026-10-13-r6-git/report.md)；[R5 报告](results/2026-10-13-r5-explorer/report.md)保留当时的截止记录。

报告：[阶段 0](../baseline/README.md)、[阶段 1](results/2026-10-13-phase1/report.md)、[阶段 2](results/2026-10-13-phase2/report.md)。后续修改必须清理 policy 中过期的例外和字段登记，不得整体刷新登记绕过门禁。

R7.1 当前机器完整 workspace：688 通过、0 失败、14 忽略（应用 591、编辑核心 68、独立域重复验证 29）。34 项测试迁移无缺失，新增行为 2 项、独立布局重复验证 13 项。架构自测 33、报告自测 3 项通过，legacy exception 文件 39。release、四项 Windows 终端 smoke 和隔离启动通过；同机修改前 31 项性能基线已冻结。固定 1.97.1、原生人工、跨平台和后续专项 A/B 保留待验。
