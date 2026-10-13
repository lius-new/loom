# Git 迁移（R6）

Git 的纯规则、进程服务、应用事务与视图已分别归入 `features/git`、
`infrastructure/git`、`app/git*` 和 `ui/git`。执行历史见
[方案](git-migration-plan.md)，验收证据见 [报告](results/2026-10-13-r6-git/report.md)。

## 状态与事务

- `GitModel` 的 7 个私有字段保存历史、提交详情缓存、固定仓库、diff 请求代次与克隆状态。
- `GitUiState` 的 26 个字段保存表单、滚动、悬停、菜单、选择器、确认框与偏好。
- 根状态保留 `GitPanelState { model, ui }` 的组合。Source Control 抽屉的宽度、可见性和拖动
  归 app chrome；diff 分栏与 hunk 预览归编辑器交互状态。
- `GitStoreSnapshot` 保持独立 reactive store。扫描在后台准备展示数据，在 app 层校验目标与
  代次后发布，不建立第二份业务状态或复制文档。
- Git 写操作继续通过 `GitTasks`：单操作互斥、后台运行、UI 回合完成、磁盘对账、冲突提示、
  错误保留提交输入、完成后重扫。只读加载、克隆和 hunk 加载保持原有调度。

## 视图接口

`ui/git/commands.rs` 的 `Binding` 组合 `StateAccess<GitPanelState>`、只读帧输入与语义命令。
投影更新只触及 Git 面板，`Snapshot` 只提供实时读取，视图没有发布扫描或修改操作状态的入口。
帧输入提供抽屉 resize 状态、是否允许 resize、终端焦点及活动 diff 的比较身份。

面板、分支选择器、stash 切换、确认框、菜单、徽标和克隆对话框都位于 `ui/git`。表单使用
`InputBinding::new_access` 与 `render_access`，继续保留选区、IME 与输入焦点状态。

跨功能工作由 `app/git_view.rs` 执行：打开文件、Changes 拖放、抽屉 resize、剪贴板与系统显示
位置、Git 操作等。标题栏由 shell 注入 Git controls；键盘动作由 app 映射到视图命令。
`app/git_peek.rs` 持有 hunk 加载线程与旧结果条件校验，视图只发出关闭、导航、暂存与丢弃命令。
`app/git_clone.rs` 接管原 workspace_actions 中的克隆事务；URL 目录规则归 feature。

只读 diff 编辑器的 Git 操作也通过同一命令接口；其文档滚动、窗格选择、工具栏和分栏交互仍是
登记到 R8 的组合适配，不把这部分声称为已完成窄接口迁移。

## 门禁

- `features/git` 禁止 GUI、根状态、进程、文件、线程与例外。
- `infrastructure/git` 只允许基础设施、feature/domain 与 Git 进程服务所需的标准库/serde。
- `ui/git` 禁止根状态、app/基础设施/旧 workspace 命令、源码包含与迁移例外。
- 根状态不得重新导出 Git 类型；模型私有字段和 UI 字段逐项登记。
- 已删除 `src/git/`、`git_actions.rs`、旧 Git UI 模块与旧 `src/editor/`，不保留路径别名。

## 验收边界

完整 workspace 包含 16 项复用生产 domain 源码的独立验证，单列统计，不算新增行为测试。
原有 Git UI 集成测试随组合接线迁到 app，保留测试名、交互步骤与断言。

`StateAccess::get` 仍会读取并克隆根值；迁移收窄的是接口。性能通过冻结 release 测试二进制，
对 10,000 文件快照准备与面板帧做交错 A/B。它不代表真实仓库进程耗时或原生窗口帧延迟。

R6.4 的执行偏差继续保留：只读加载未接入可注入执行器；工作区切换后的写操作完成仍沿用旧
应用规则，没有增加按启动工作区拒收的行为。暂停策略是停止周期扫描，已经开始的扫描仍可
发布，显式刷新或目标变化仍唤醒暂停的 poller；这一点更正原方案“暂停期间不发布”的描述。
补充测试覆盖暂停中的在途代次、取消固定仓库后旧扫描被拒以及当前目标扫描继续生效。

本机自动验收不能替代提交、丢弃确认、分支/stash、worktree、克隆、diff 拖动与 hunk 预览的
原生人工验收；macOS/Linux、远端 CI、安装包验收仍待执行。
