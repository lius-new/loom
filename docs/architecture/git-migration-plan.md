# 阶段 6（R6）迁移方案：Git

2026-10-13 起草，基础提交 e6eb1e3（R5 完成），分支 `refactor/r6-git`。本文是执行前方案，
完成后另写 `git-migration.md` 与 `results/<date>-r6-git/report.md`。

## 目标与不做的事

目标：把 Git 的纯规则、进程/运行时适配、业务与界面状态、命令与后台任务、轮询接线和 Git
视图分别放到 `features/git`、`infrastructure/git`、`app/git*`、`ui/git`，删除顶层
`src/git/` 与 `git_actions.rs`，并让 Git 功能与视图进入无例外门禁。

不做：
- 不改 Git 命令行参数、解析规则、轮询间隔、暂停策略和错误文案；不引入 libgit2。
- 不改 Git 面板、分支选择器、diff 视图的视觉与交互。
- 不拆 AppState 为多个 reactive store；`GitStoreSnapshot` 继续是独立 `State`。
- 不迁移终端（R7）、设置页与快捷键（R8）；设置持久化格式不变。

## 现状盘点

| 区域 | 规模 | 内容 |
| --- | --- | --- |
| `src/git/` | 约 5 700 行，34 测试 | types/parser/diff/error 为纯逻辑；backend、command、runtime、discovery 启动进程或读盘；store.rs 同时含 `GitStoreSnapshot`（纯）与 `GitService`/`PollControl`（线程、进程） |
| `src/git_actions.rs` | 2 685 行，11 测试 | 约 60 个命令：暂存/提交/丢弃/冲突/暂存区/历史/分支/检出/同步/worktree/克隆；`run_with` 统一后台执行，结果回 UI 线程 |
| `src/ui/git_*.rs` 等 | 约 6 800 行，27 测试 | git_panel 3 280、branch_picker 1 531、git_confirm、git_menu、stash_switch、git_badge、clone_repository；全部直接读写 AppState |
| `src/model/diff_document.rs` | 766 行 | 由 Git diff 内容构建的只读文档，`GitStoreSnapshot.diffs` 缓存它；唯一 UI 依赖是默认行高读取字体 |
| `src/editor/hunk_peek.rs` | — | 编辑器内 hunk 预览，调用 git_actions |
| `src/app/shell.rs` | — | Git 轮询 effect、扫描结果拒收、历史加载、通知；`git_refresh_tests` |

policy：根字段 38 项登记 `remove_in: R6`；editor 子状态 `editor_hunk_peek`；16 个例外文件
（src/git 下 6 个、git_actions、6 个 Git 视图、hunk_peek、workspace_actions 的克隆部分）；
runtime 桥接 git_actions、git/backend、git/store。

### 字段归属判断

| 字段 | 外部使用 | 建议归属 |
| --- | --- | --- |
| `show_source_control`、`git_sidebar_w`、`resizing_git_sidebar` | shell 布局、状态栏、layout_map、搜索、workspace_home | **左侧抽屉 chrome**，与 R5 的右侧抽屉同理归 app/state |
| `git_split_drag`、`git_split_ratio` | diff_editor 的左右分栏拖动 | **diff 视图交互**，归 editor 交互状态（R3 的 editor_interaction） |
| `git_inline_blame`、`git_split_diff`、`git_tree_view` | 设置页、settings.json、diff 视图 | Git 视图偏好，归 Git UI 状态；settings.json 格式不变，由 app 映射读写 |
| `git_diff_loading`、`git_open_diff_sequence` | 状态栏 | Git 模型（打开 diff 的请求代次） |
| `git_pinned_repository` | Explorer 的 forget_worktree、shell 轮询目标 | Git 模型，外部经 app 命令修改 |
| 其余 31 项 | 仅 Git 代码 | Git 模型或 Git UI 状态 |

## 目标结构

```text
src/features/git/                 无进程、文件、线程、lgui、AppState
  types.rs error.rs parser.rs diff.rs
  diff_document.rs                原 model/diff_document，行高由调用方提供
  snapshot.rs                     GitStoreSnapshot、展示数据、路径装饰、PollTarget
  ports.rs                        GitBackend（发现、状态、克隆）
  model.rs                        GitModel：历史、提交详情缓存、展开仓库、固定仓库、diff 请求代次、克隆请求
  tests.rs
src/infrastructure/git/
  command.rs runtime.rs discovery.rs backend.rs
  service.rs                      GitService、PollControl、轮询线程、service()
src/ui/git/
  state.rs                        GitUiState（输入、悬停、滚动、菜单、选择器、确认框、偏好）与 GitPanelState
  commands.rs                     语义命令与帧输入
  panel.rs branch_picker.rs stash_switch.rs confirm.rs menu.rs badge.rs clone.rs
src/app/
  git.rs                          命令与跨功能事务
  git_tasks.rs                    后台执行、单操作互斥、完成后磁盘对账与刷新请求（可注入执行器）
  git_poll.rs                     轮询目标、扫描结果拒收与发布
  git_view.rs                     投影组装与命令执行
```

依赖方向：`app/git* → {features/git, infrastructure/git, ui/git}`；`infrastructure/git →
features/git`；`ui/git → features/git`（只读类型）。Explorer、编辑器、状态栏只经 app 取得
Git 展示数据或发出命令。

## 分步执行

每步结束运行完整测试、fmt、CI 参数 clippy、两条门禁命令与隔离启动 smoke，并单独提交。

### R6.1 纯 Git 规则迁入 features/git

- 迁移 types、error、parser、diff、snapshot（`GitStoreSnapshot` 及其纯方法、`PollTarget` 数据）与 `GitBackend` 端口；`DiffDocument` 从 model 迁入，默认行高改由调用方传入。
- `src/git/` 中剩余的进程/线程部分暂时直接引用 `features::git`（登记依赖），R6.2 整体迁走。
- 全仓 `crate::git::` 路径按类型改为 `crate::features::git::`；不保留 re-export。
- 门禁：`features/git` 只依赖 features/domain，禁止根状态/GUI 类型、进程、文件与线程，不得有例外。
- 测试：parser/diff/types/error/snapshot 原测试按原名迁入。

### R6.2 进程与运行时适配迁入 infrastructure/git

- 迁移 command、runtime、discovery、backend 与 store 中的 `GitService`/`PollControl`/轮询线程、`mod.rs` 的 `service()`/`start_polling`；删除 `src/git/`。
- 门禁：`infrastructure/git` 只允许 features/domain、std::fs/process/thread 与运行时清单需要的 serde。
- runtime 桥接登记随文件更新；不改变进程策略、取消与超时。

### R6.3 状态归属

- AppState 删除 38 个 Git 字段，新增 `git: GitPanelState { model, ui }` 与 `source_control: SidebarChrome`（visible/width/resizing）；diff 分栏两项迁入 editor 交互状态。
- 新增 `git_ownership` 逐字段登记，门禁检查字段一致且模型字段私有。
- settings.json 中三项 Git 偏好的读写改由 app 映射到 `git.ui`，夹具往返一致。

### R6.4 命令与后台任务

- `git_actions.rs` → `app/git.rs` + `app/git_tasks.rs`。后台执行器可注入（同 `app/search_tasks`），测试可控制完成顺序。
- 保持：同一时刻只有一个写操作、完成后磁盘对账并对冲突提示、失败写入 `last_error`、结束后请求刷新、提交成功清空输入。
- 新增测试：操作进行中拒绝第二个操作、失败保留输入、完成晚于工作区切换时不写入新工作区、有未保存文件时拒绝提交/丢弃。

### R6.5 轮询与快照发布

- shell 中的 Git 轮询 effect、目标计算、`apply_git_scan` 与历史加载迁入 `app/git_poll.rs`；`git_refresh_tests` 按原名迁移。
- 新增竞态测试：根目录或活动文件变化后晚到的扫描被拒、暂停策略与固定仓库在扫描中被移除。收尾核对原策略：暂停阻止周期扫描，在途结果与显式刷新仍允许；保留这一行为，不新增暂停期间拒收规则。
- 性能：记录一次大仓库快照 `prepare_for_ui` 与面板渲染的前后对照（同 R5.4 的冻结二进制交错方式）。

### R6.6 Git 视图窄接口

- Git 面板、分支选择器、stash 切换、确认框、Git 菜单、徽标、克隆对话框迁入 `ui/git`，只接收 `StateAccess<GitPanelState>`、只读 `GitStoreSnapshot` 与帧输入（活动文件、未保存路径集合等）和语义命令。
- hunk 预览改为发出编辑器/Git 命令；git_menu、git_panel 不再调用 `app::explorer`（抽屉文件打开改为命令）。
- 视图规模大，可按 R6.6a（面板）与 R6.6b（选择器、对话框、菜单、徽标、hunk 预览）分两次提交。

### R6.7 门禁、清理与文档

- 删除所有过期例外、根状态适配器与 runtime 桥接登记；`workspace_actions.rs` 的克隆部分迁出后其 remove_in 只剩 R8。
- 门禁：`ui/git` 禁止根状态与例外；根状态不得 re-export Git 类型；Git 字段登记完整。
- 文档：`git-migration.md`、R6 报告、进度页。

## 验收清单

- 完整 workspace 测试 0 失败，R5 的 650 条清单全部保留并映射，新增用例单列。
- fmt、CI 参数 clippy、门禁与自测、release 构建、隔离启动/恢复 smoke。
- 带真实 Git 的现有测试（backend、store、git_actions）继续在本机 Git 下通过。
- 性能对照保留原始样本；不为数字调整算法。
- 原生人工：提交、暂存、丢弃确认、分支切换与 stash、worktree、克隆、diff 视图拖动、hunk 预览，仍需人工验收。

## 风险与对策

| 风险 | 对策 |
| --- | --- |
| `crate::git::` 引用遍布全仓 | R6.1/R6.2 用编译错误驱动的脚本改路径，每步提交前全量编译与测试 |
| `GitStoreSnapshot.diffs` 缓存 `DiffDocument`，迁移时触到编辑器 | DiffDocument 只改模块路径与行高参数，行为测试覆盖 |
| 后台任务与 UI 线程的交错顺序 | 可注入执行器 + 手动完成顺序测试，保留现有代次/目标校验 |
| 面板文件大、交互多 | 先迁状态与命令，视图最后迁；面板拆两次提交 |
| 设置格式回归 | settings 夹具往返测试 |
| 进程策略被无意修改 | infrastructure/git 原样搬运，只改模块路径 |

## 待决定事项（按推荐方案执行，R6.3 前可调整）

1. 左侧 Source Control 抽屉的显示、宽度、拖动归 app chrome（推荐，与 R5 一致）。
2. diff 视图左右分栏的拖动与比例归编辑器交互状态，而不是 Git（推荐）。
3. 三项 Git 视图偏好归 Git UI 状态，settings.json 格式不变（推荐）。
4. 删除 `src/git/`，不保留 re-export（推荐，符合项目偏好）。

## 执行记录

### R6.1（2026-10-13，本机自动验收）

- 新增 `features/git/{types,error,parser,diff,diff_document,snapshot}.rs`：前四个从 `src/git/` 原样迁入；`DiffDocument` 从 `model/` 迁入；`snapshot.rs` 从 `store.rs` 拆出 `GitStoreSnapshot`、展示数据、`PollTarget` 与扫描比较/行变更收集/操作状态等纯函数。`src/git/` 只剩 command、runtime、discovery、backend 与 store 的 `GitService`/`PollControl`。
- `DiffDocument` 默认行高原读取编辑器字体：改为 `ui::diff_editor::row_h()` 计算并由 app/UI 使用，feature 只保留 `row_height_for(font_size)`。
- 全仓 `crate::git::` 引用按类型改到 `crate::features::git`（分组 import 自动拆分），不保留 re-export。
- 与方案的偏差：`GitBackend` 端口只描述扫描（发现/状态/克隆），唯一使用者是 `GitService`，因此随 R6.2 留在 infrastructure，不进 feature；方案中的 `features/git/ports.rs` 取消。
- 门禁：`features/git` 只依赖 features/domain，禁止根状态/GUI/进程类型与例外；新增负例（自测 24 项）。policy：删除 model/diff_document 例外（legacy exception 文件 57 → 56）；7 个 Git/diff 视图与 key_actions 去掉 `crate::git` 依赖（只用类型）；src/git 剩余文件与 git_actions、hunk_peek 登记 `crate::features`。
- 测试：store 的 7 项快照测试按原名迁入 snapshot，4 项 PollControl 测试留在 store；types/error/parser/diff/diff_document 测试随文件迁移，名称不变。
- 结果：完整 workspace 650 通过、0 失败、13 忽略（与 R5 相同，纯迁移无新增）；fmt、CI 参数 clippy、门禁与 24 项自测通过；隔离启动/恢复 smoke 通过。

### R6.2（2026-10-13，本机自动验收）

- `src/git/` 删除：command、runtime、discovery、backend 原样迁入 `infrastructure/git/`，`store.rs` 改名 `service.rs`（`GitService`、`PollControl`、轮询线程），`mod.rs` 保留 `service()`/`start_polling`/`PollingHandle`。全仓引用改为 `crate::infrastructure::git`，不保留别名。
- Git 运行时访问器 `runtime()` 改名 `git_runtime()`：原名与 `AppRuntime` 桥接的词法检查冲突，使 backend/store 被登记为 runtime 桥接；改名后两项登记删除，实际不涉及应用运行时。
- 门禁：`infrastructure/git` 只允许 infrastructure/features/domain、std::fs/process/thread 与 serde，禁止根状态/GUI 与例外；新增负例（自测 25 项）。policy：删除 src/git 6 个例外（legacy exception 文件 56 → 50）与 2 个 runtime 桥接；git_actions、hunk_peek、workspace_actions 的 `crate::git` 依赖改登记为 `crate::infrastructure`。
- 结果：完整 workspace 650 通过、0 失败、13 忽略（纯迁移）；fmt、CI 参数 clippy、门禁与 25 项自测通过；隔离启动/恢复 smoke 通过。进程参数、超时、取消与运行时解析未改动。

### R6.3（2026-10-13，本机自动验收）

- AppState 删除 38 个 Git 字段，新增 `git: GitPanelState { model, ui }` 与 `source_control: SidebarChrome`（visible/width/resizing）；EditorState 新增 `diff_split: DiffSplit`（drag/ratio，比例默认 0.5）。根字段登记 76 → 40，editor 30 → 31。
- 新增 `features/git/model.rs`：`GitModel` 字段私有，持有历史、展开的提交、提交详情缓存（上限 64）、打开 diff 的请求代次与加载请求、用户固定的仓库、克隆进行中标记；`GitHistory`、`ExpandedCommit` 从 git_panel 迁入。方法覆盖原全部读写：`begin/cancel/finish/abandon_diff_open`、`toggle_commit`/`set_commit_files`、`cache_commit_detail`、`pin_repository`/`pinned_for`/`unpin`、`begin/finish_clone`。
- 新增 `ui/git/state.rs`：`GitUiState` 26 项（悬停、滚动、提交表单、菜单、确认框、提交卡片、分支选择器、stash 切换、克隆对话框与三项视图偏好）；折叠分区默认值保持（Worktrees/Stashes/History 折叠）。
- settings.json 格式不变：`settings()`/`apply_settings` 把 git_inline_blame/split_diff/tree_view 映射到 `git.ui`；会话字段 `source_control_open` 不变。
- 登记：`git_ownership` 33 项（模型 7、UI 26）；`source_control` 与 `diff_split` 写明由 features/git 更正为 app chrome / 编辑器交互。门禁把 Explorer 的字段登记与模型私有检查推广到 GitModel/GitUiState，新增负例（自测 26 项）。
- 测试：6 项模型测试（diff 打开代次与取消、提交展开/折叠与晚到文件、固定仓库随活动文件失效、详情缓存上限、克隆互斥）；2 项状态测试（三项偏好经 settings 往返、chrome 与 diff 分栏默认值）。
- 结果：完整 workspace 658 通过、0 失败、13 忽略（应用 574，编辑核心 68，独立域 16）；fmt、CI 参数 clippy、门禁与 26 项自测通过；隔离启动/恢复 smoke 通过。

### R6.4（2026-10-13，本机自动验收）

- `src/git_actions.rs` → `app/git.rs`（约 60 个 Source Control 命令，名称不变），调用方改为 `crate::app::git::`，不保留别名。
- 新增 `app/git_tasks.rs`：`GitExecutor`（后台 spawn + UI 线程 post）与 `GitTasks::run`，承接原 `run_with` 的全部规则——同一时刻一个写操作、开始时标记并清除上次错误、后台完成后在 UI 线程应用结果、提交成功清空输入、磁盘对账与冲突提示、失败写 `last_error` 且保留输入、无论成败请求重扫、worker 无法启动时撤销标记。`app/git.rs` 的 `run_with` 只组装 Git 服务调用并委托。
- 与方案的偏差：只读加载（历史、提交文件、提交卡片、打开 diff、分支列表）仍各自起线程，未接入执行器；它们已有各自的代次/目标校验，本轮不改调度。方案中“完成晚于工作区切换时不写入新工作区”不是现有行为（写操作完成总会应用），未新增该规则；“有未保存文件时拒绝提交/丢弃”已有原测试覆盖。
- 登记：runtime 桥接由 git_actions 改为 app/git、app/git_tasks；删除 git_actions 例外（legacy exception 文件 50 → 49）；Git 视图与 hunk 预览的 `crate::git_actions` 依赖改为临时 `crate::app`，R6.6 移除。
- 测试：git_actions 的 11 项测试随文件迁移、名称不变；新增 5 项 git_tasks 测试（第二个操作被拒、完成只在 UI 回合应用并清空提交输入、失败保留输入并记录错误、worker 无法启动撤销标记、新操作清除旧错误）。
- 结果：完整 workspace 663 通过、0 失败、13 忽略（应用 579，编辑核心 68，独立域 16）；fmt、CI 参数 clippy、门禁与 26 项自测通过；隔离启动/恢复 smoke 通过。

### R6.5（2026-10-13，本机自动验收）

- 新增 `app/git_poll.rs`：`poll_target`（活动文件、打开的 diff、在活动文件未变时仍有效的固定仓库）、`history_key`、`start`（2 s 轮询）、`deliver` 与 `apply_scan`。shell 只保留 effect 接线，原 `apply_git_scan`、`GitScanNotification` 与 `git_refresh_tests` 删除，两项测试按原名迁入。
- **修复既有缺陷**：轮询器发布的目标包含固定仓库，而原 `apply_git_scan` 用不含固定仓库的 `workspace.git_poll_target()` 比较，因此在 Source Control 中选定仓库后、活动文件变化前，所有扫描都被当作过期丢弃（暂存等操作后状态不刷新）。现在设置轮询目标与校验扫描使用同一个 `poll_target`。回归测试在恢复旧比较时失败（已验证），修复后通过。
- 新增测试：固定仓库期间扫描持续生效、固定前开始的扫描被拒且固定随活动文件变化失效、旧代次不覆盖新代次、历史仅在抽屉与历史分区可见时跟随活动仓库。
- 性能：本步未改动快照准备、轮询线程或渲染路径，前后对照没有可比较的差异；面板渲染对照放在改动视图的 R6.6。
- 结果：完整 workspace 667 通过、0 失败、13 忽略（应用 583，编辑核心 68，独立域 16）；fmt、CI 参数 clippy、门禁与 26 项自测通过；隔离启动/恢复 smoke 通过。

### R6.6（2026-10-13，本机自动验收）

实现检查点：`e02c7b7`。

- 七个 Git UI 模块与 hunk 渲染迁入 `ui/git`；`Binding` 组合 Git 面板投影、实时只读 `Snapshot`、帧输入与语义命令。视图不再访问 AppState、app 命令或进程服务。
- `app/git_view` 执行 Git 与跨功能命令，处理 Changes 拖动、文件打开、chrome resize、剪贴板与显示位置；shell 注入标题栏 Git controls，键盘接线归 app。
- hunk 加载线程迁入 `app/git_peek`，原结果条件校验保留；克隆事务迁入 `app/git_clone`，URL 规则归 feature。diff 编辑器的 Git 操作使用同一命令接口，剩余文档/窗格交互归 R8。
- 原 Git UI 集成测试与 URL/hunk 测试共 30 项保留名称并映射；新增 6 项投影、跨功能事务、无根状态 hunk 点击与固定仓库/暂停竞态回归。新增 1 项 ignored Git 性能基准。
- 保持 R6.4 的已记录执行偏差；暂停策略按原实现校准，并有专用测试。

### R6.7（2026-10-13，本机自动验收）

- 删除八个旧模块的例外、七个 UI 根状态适配器与旧 hunk runtime 桥接；登记新 app 加载桥接。workspace_actions 的 remove_in 只剩 R8；Git 三层没有迁移例外。
- 门禁增加 Git 视图禁止根状态/服务/源码包含/例外、根状态禁止重新导出 Git 类型，架构负例 26 → 29；报告自测 3 项通过。
- 最终完整回归 673 通过、0 失败、14 忽略（应用 589、编辑核心 68、独立域重复验证 16）；fmt、严格 CI Clippy、release 构建、隔离 Git 回归、四项 ConPTY smoke、隔离启动和无参数恢复均通过。
- 31 项常规性能指标补跑；Git 冻结二进制交错 A/B（10,000 文件，预热 + 各 10 轮）：全量准备 −1.94%，面板帧 +2.77%，节点均为 278；不声称性能提升或原生帧延迟改善。
- 首次隔离回归遇到现有 ConPTY scrollback 断言波动，单项与最终完整回归通过；首次失败日志保留。详见 [验收报告](results/2026-10-13-r6-git/report.md)。
