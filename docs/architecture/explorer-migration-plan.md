# 阶段 5（R5）迁移方案：Explorer

2026-10-13 起草，基础提交 a29abb5（R4 检查点）。本文是执行前方案，完成后另写
`explorer-migration.md` 与 `results/<date>-r5-explorer/report.md`，不覆盖本文。

## 目标与不做的事

目标：把 Explorer 的目录缓存、根目录/最近目录、新建/重命名/删除流程、右键菜单与树视图
状态从 AppState 与 `workspace_actions.rs` 中拆出，形成与 R2 Search 同形的
`features/explorer` + `ui/explorer` + `app/explorer` + `infrastructure/explorer_filesystem`，
并把 Explorer 纳入无例外门禁。

不做：
- 不拆 AppState 为多个 reactive store，不解决 `State::get` 的根克隆（仍按 R8 后续处理）。
- 不迁移关闭确认/保存（`request_close_*`、`save_*`、会话包装）——它们属于 workspace
  命令，留在 `workspace_actions.rs`，后续单独整理。
- 不迁移 `clone_repository`（R6 Git）。
- 不改会话 JSON 格式、配置路径、文件读取上限、watcher 调度和 300 ms 指示器规则。
- 不改树的视觉、排序、懒加载、粘性目录行和滚动条行为。

## 现状盘点

| 来源 | 规模 | R5 相关内容 |
| --- | --- | --- |
| `src/workspace_actions.rs` | 2429 行，38 测试 | 打开/拖放/定位（231–480）、对话框与新建/重命名/删除/回收站/剪贴板（509–1075）、文件夹装载/移除/启动参数/hydrate/最近目录（1193–1400） |
| `src/ui/sidebar.rs` | 1483 行，5 测试 | 树渲染、header、空状态、新建/重命名行、滚动条、宽度拖动；直接 `State<AppState>`，且在 UI 中调用 `read_directory` |
| `src/ui/context_menu.rs` | 753 行，7 测试 | 菜单项与可用性、目标解析、`reveal_in_file_manager`（平台 I/O 在 UI 里） |
| `src/file_tree.rs` | 225 行，4 测试 | 读目录、刷新已加载目录、编辑中延迟刷新、剪枝 |
| `src/file_watcher.rs` | 817 行，11 测试 | `prepare_batch` **克隆整个 AppState** 只为跑一遍树刷新；`apply_prepared_batch` 用 `Arc::ptr_eq` 拒收过期列表 |
| `src/ui/welcome.rs`、`workspace_home.rs` | 根状态适配器 | 读 `recent_folders`，调用 `choose_*`/`open_recent_folder` |

policy 中 `remove_in: "R5"` 的根字段 24 项；相关登记还有 root_state_adapters
（sidebar、context_menu、welcome、workspace_home、tab_context_menu、terminal_tab_context_menu）、
runtime_bridges（workspace_actions）以及 file_tree/file_watcher/workspace_actions 的依赖例外。

### 字段外部使用（决定归属的关键）

| 字段 | Explorer 外的读写方 | 结论 |
| --- | --- | --- |
| `workspace_folders` | search 根、git 根 effect、project_settings、terminal cwd、会话、watcher 根比对、vim 测试 | 业务上是“工作区根”，Explorer 拥有但必须提供只读查询；外部只经 app 取值 |
| `recent_folders` | 会话保存/恢复、welcome | Explorer 模型拥有，welcome 接收只读切片 |
| `show_drawer`、`drawer_mode` | search、keymap context_stack、state::apply、file_loading、shell | **抽屉是 Files/Search 共用 chrome**，不应进 Explorer |
| `sidebar_w`、`resizing_sidebar` | shell 布局 | 同上，抽屉宽度属于 chrome |
| `context_menu`、`context_menu_target` | tabs、terminal、editor 打开其他菜单时清除；keymap 判断遮罩 | Explorer UI 拥有，外部改为调用 app 级 `dismiss_overlays` / 查询 |
| `pending_file_tree_refresh`、`dir_entries`、`expanded` | file_watcher、shell effect | Explorer 模型拥有，watcher 通过模型 API |
| 其余（hover、滚动、滚动条拖动、create/rename 输入与错误） | 仅 Explorer 与 shell 的 hover 接线 | Explorer UI 状态 |

**需要在 policy 中显式改 owner 的 4 项**：`show_drawer`、`drawer_mode`、`sidebar_w`、
`resizing_sidebar` 从 `features/explorer/private model` 改为 `app/state (drawer chrome)`，
理由写入 policy 与报告。这是有依据的归属更正，不是批量刷新登记。

## 目标结构

```text
src/features/explorer/            # 无 lgui/AppState/Workspace/InputState/std::fs
  mod.rs
  types.rs        DirEntry、TargetKind、ContextTarget、CreateKind、CreateRequest、RenameRequest
  tree.rs         TreeCache：dir_entries(Arc 列表)、expanded；插入/折叠/剪枝/重定位、可见行、reveal 索引
  roots.rs        WorkspaceRoots：roots + recents；replace/add/remove/rebase/remember/hydrate(经端口过滤)
  edit.rs         新建/重命名名称校验（invalid segment、保留名、同名冲突经端口查询）
  refresh.rs      刷新计划：受影响父目录、全量已加载目录、编辑中延迟、剪枝
  model.rs        ExplorerModel（私有字段）+ 转换方法，返回 Outcome{retired, persist_roots, ...}
  ports.rs        DirectoryPort{read_directory, exists, is_dir}；FileOpsPort{create, rename, trash, clipboard, reveal}
  tests.rs        内存端口测试
src/infrastructure/explorer_filesystem.rs
                  read_directory、create/rename、move_path_to_trash（Win/mac/Linux）、
                  set_file_clipboard、reveal_in_file_manager
src/ui/explorer/                  # 不接收 AppState/AppRuntime/Workspace
  state.rs        ExplorerUiState：tree_scroll(_x)、两个滚动条拖动、hovered/tree_hovered_path、
                  menu{position, target, hover}、create_input(InputState)、create_error
  commands.rs     Command 枚举 + Commands 回调（同 ui/search/commands.rs）
  view.rs         原 sidebar.rs 的树/行/粘性/滚动条
  header.rs       drawer_header、empty_state
  context_menu.rs 原 context_menu.rs 的绘制与 action_enabled
src/app/
  explorer.rs     命令分派、跨功能事务、对话框、启动参数、persist 调用
  explorer_tests.rs
  state.rs        AppState.explorer: ExplorerPanelState{model, ui}；AppState.drawer: DrawerChrome
```

依赖方向：`app/explorer → {features/explorer, ui/explorer, infrastructure/explorer_filesystem, app::Workspace}`；
`infrastructure/explorer_filesystem → features/explorer`（实现端口）；
`ui/explorer → features/explorer(只读类型)`。Search/Git/Terminal 不依赖 `features/explorer`，
它们拿到的根目录仍是 app 组装的 `Vec<PathBuf>`（Search 已经如此）。

### 跨功能事务留在 app

以下操作同时改 Explorer 与 Workspace，必须在**一次根 State 更新**里完成，由 `app/explorer.rs` 编排：

| 操作 | Workspace 部分 | Explorer 部分 |
| --- | --- | --- |
| 重命名 | `move_open_paths_under` | roots/recents rebase、缓存剪枝与重读父目录、清编辑态、按需 persist |
| 删除（回收站） | `has_dirty_paths_under` 预检、`close_paths_under` | roots/recents 剪枝、缓存剪枝、清编辑态、persist |
| 移除根 / forget_worktree | — / 清 `git_pinned_repository` | remove_root、清编辑态、滚动归零 |
| 打开树文件 / 拖放到 pane / reveal | 打开或激活 tab、pane drop | 展开祖先、计算可见行并设置滚动 |
| 装载/添加文件夹 | — | replace/add、`show_drawer = true`、清 welcome hover 和 toast |

模型方法只返回待回收数据和“是否需持久化根”；回收交给既有 `ui_dispatch::retire`，
持久化交给 app 的 `persist_workspace`。I/O（读目录、rename、trash）在调用模型前后由 app
经 infrastructure 端口完成，模型接收结果。

## 分步执行

每步结束都跑 `cargo test --workspace`、fmt、clippy 与两条门禁命令；每步可单独回滚。

### R5.1 类型、端口与纯模型

- 新建 `features/explorer`，`DirEntry` 等类型从 `app/state.rs` 迁入；`crate::state::DirEntry` 等保留 crate 内别名一步，R5.6 删除。
- `file_tree.rs` 的逻辑改写为 `TreeCache` + `refresh.rs`，读盘经 `DirectoryPort`。
- `validate_create_path`、`validate_rename_path`、`invalid_file_name_segment`、`existing_unique`、`normalized_folder`、`remember_folder` 迁入 `edit.rs`/`roots.rs`。
- `infrastructure/explorer_filesystem.rs` 实现端口；平台 trash/clipboard/reveal 从 workspace_actions 和 context_menu 迁出。
- 测试：内存端口覆盖排序、懒加载、剪枝、延迟刷新、rename rebase、删除剪枝、名称校验；原 file_tree 4 项测试改为模型测试并在清单中映射。

验收：AppState 尚未改动，行为零变化；新目录无例外。

### R5.2 状态归属：ExplorerPanelState 与 DrawerChrome

- AppState 删除 24 个字段，新增 `explorer: ExplorerPanelState` 与 `drawer: DrawerChrome{visible, mode, width, resizing}`。
- 登记：根字段 98 → 76（−24 +2）；新增 `explorer_ownership`（model/ui 逐字段）与 drawer 4 项 owner 更正。
- 机械替换所有读写点；外部清菜单处（tabs、terminal ×2、editor、state::apply）改为 `app.explorer.ui.dismiss_menu()`，keymap 改为 `app.explorer.ui.menu_open()` / `app.drawer` 查询。
- 会话：`workspace_session.rs` 从 `explorer.model.roots()/recents()` 读写，JSON 字段名不变；用 `docs/baseline/fixtures/session-*.json` 回归。

验收：615 测试不变通过；夹具往返一致。

### R5.3 应用命令与跨功能事务

- 新建 `app/explorer.rs`，迁入 workspace_actions 中 Explorer 相关函数（open_tree_file、finish_file_drag、finish_drawer_file_drag、reveal_file_in_tree、choose_*、begin/finish/cancel create/rename、copy、delete、remove/add/load folder、forget_worktree、open_recent_folder、open_launch_paths、hydrate_workspace_folders）。
- `git_actions.rs` 的 `move_path_to_trash`/`apply_deleted_path_state`/`add_folder`/`forget_worktree` 改调 app/explorer 与 infrastructure；`file_loading.rs` 的 `open_tree_file` 同。
- `workspace_actions.rs` 只剩关闭/保存/会话包装与 clone_repository；从 runtime_bridges 检查是否仍需登记，不需要就移除。
- 测试：workspace_actions 中 Explorer 相关测试按原名迁到 `app/explorer_tests.rs`，生成 `test-migration.json` 映射。新增：rename 期间晚到 watcher 结果、删除含脏文档被拒、移除根时清理指向该根的 create/rename、Unicode 文件名的新建/重命名。

### R5.4 watcher 接入与去掉整 AppState 克隆

- `prepare_batch` 改为克隆 `TreeCache`（Arc 列表的 map，开销低）和 roots，而不是 `app.clone()`；输出结构不变。
- 保留语义：根变化时丢弃整批；编辑中或 batch 期间开始编辑 → 置 pending；`Arc::ptr_eq` 不等 → retry；删除的目录同时移出 expanded；旧列表交给回收器。
- 新增测试：准备期间用户展开/折叠目录、准备期间开始新建、根在准备期间被替换，三种都不得覆盖新状态。
- 性能：在 R4 的 31 指标外，记录大目录树 + 批量变更的 `prepare_batch` 耗时前后对照（同机交错采样，按 R4 方式保留原始比值）。

### R5.5 UI 窄接口

- `sidebar.rs` → `ui/explorer/{view,header}.rs`；`context_menu.rs` → `ui/explorer/context_menu.rs`。
- view 输入：`StateAccess<ExplorerPanelState>`、只读 `GitStoreSnapshot`（装饰色）、只读文件拖动状态、`Commands`。header 的抽屉切换走 Command 由 app 改 `drawer`。
- **去掉 UI 内读盘**：sidebar 展开目录时调用 `read_directory` 改为发出 `Command::Expand(path)`，由 app 经端口读后装入模型（同步读，时序不变；不在本轮改成异步）。
- 右键菜单的 `OpenTerminal` 需要 TerminalTabs：改为 `Command::OpenTerminal(dir)`，由 app 接线，菜单不再接收 `State<TerminalTabs>`。
- welcome / workspace_home 接收 `recent_folders: Arc<[PathBuf]>` 与命令回调，从 root_state_adapters 移除。tab_context_menu 的 `reveal_file_in_tree` 改为命令；它与 terminal_tab_context_menu 的其余适配不在 R5 范围，保留登记。
- shell 中 sidebar hover / 宽度拖动接线改为调用 Explorer UI 方法与 `drawer`。
- 保留 sidebar/context_menu 原 12 项 UI 测试，补：键盘焦点下新建输入的 IME preedit 不提交、菜单在其他菜单打开时关闭。

### R5.6 门禁、清理与文档

- `check.mjs` 增加：
  - `features/explorer` 只允许 `features`、`domain`；禁止 `AppState/AppRuntime/Workspace/ApplicationHandle/InputState/UiRect`、lgui 与 `std::fs`/`std::process`。
  - `ui/explorer` 禁止 `AppState/AppRuntime/Workspace/ApplicationHandle`。
  - `infrastructure/explorer_filesystem` 允许 `features`、`domain`、`std::fs`、`std::process` 与平台 API。
  - 两个 Explorer 目录不得有 exceptions。
  - `check.test.mjs` 为每条规则加负例。
- 删除：`file_tree.rs`、旧 `ui/sidebar.rs`、`ui/context_menu.rs`（或按 R2 做法留迁移说明且不编译）、crate 别名、过期 exceptions/adapters/bridges；legacy exception 文件数应从 60 下降。
- 更新 README 的 Explorer 相关说明（如有路径）、`progress.md`、新写 `explorer-migration.md` 与 R5 报告。

## 验收清单

- 完整 workspace 测试：0 失败，原 627 条清单（含 ignored）全部保留并映射，新增用例单列。
- 49+ 项 baseline 检查 exit 0（fmt、严格 clippy、门禁及自测、debug/release/隔离 Git 回归、ConPTY、性能六组）。
- 根字段登记 76、Explorer 逐字段登记完整；Explorer 两个目录无例外。
- 会话夹具往返、隔离 APPDATA 启动/保存/正常退出 exit 0。
- 性能：31 指标与 R4 同机比较，超过 +10% 的项做冻结二进制交错对照，原始比值保留。
- 原生人工（IME 新建/重命名输入、右键菜单、拖放到 pane、回收站、系统剪贴板、在资源管理器中显示）、macOS/Linux、远端 CI 仍列为待验，不以本机测试替代。

## 风险与对策

| 风险 | 对策 |
| --- | --- |
| `workspace_folders` 的 Vec 相等被 effect 依赖（git 根、watcher 根比对） | `roots()` 返回同顺序切片，app 传 `to_vec()`；保持 normalized_folder 规则不变，加相等性回归 |
| watcher 拒收依赖 `Arc::ptr_eq` 身份 | TreeCache 存 `Arc<Vec<DirEntry>>`，克隆只复制 Arc，测试断言身份保留 |
| 菜单清除点分散，漏改导致菜单不关 | R5.2 用编译错误驱动替换；补“打开 tab/终端菜单后 Explorer 菜单关闭”测试 |
| create/rename 输入是 UI 组件状态 | 放在 `ExplorerUiState`，模型只持请求和错误语义；门禁禁止模型引用 InputState |
| 平台 trash/clipboard/reveal 迁移改变行为 | 原样搬运，包括 Windows shell 路径转换；不改错误文案 |
| 抽屉 4 字段 owner 更正被视为绕过门禁 | 在 policy 中逐项写理由与新 owner，报告中单列 |
| 工作量集中在 workspace_actions（162 处引用） | 按 R5.3 函数清单分批迁移，每批单独编译测试 |

## 待决定事项

1. 抽屉 4 字段归 app chrome（推荐）还是按原 policy 进 Explorer。
2. 旧文件按 R2 做法保留为不编译的迁移说明，还是直接删除（项目偏好破坏性变更，推荐直接删除）。
3. 展开目录的读盘本轮保持同步（推荐，零时序变化），还是顺带改成后台加载。

## 执行记录

### R5.1（2026-10-13，本机自动验收）

- 新增 `features/explorer/{types,ports,tree,roots,edit,tests}.rs` 与 `infrastructure/explorer_filesystem.rs`。Explorer 类型从 `app/state.rs` 迁出，`crate::state` 暂以 re-export 保留旧路径（R5.6 删除）。
- `file_tree.rs` 只剩根状态桥接：用 `TreeCache::from_parts/into_parts` 临时搬运 `dir_entries/expanded`，编辑中延迟刷新仍在桥接层；R5.2 改由模型持有后删除。
- 命名校验、最近目录 MRU/hydrate、最深根匹配迁入 feature；`MAX_RECENT_FOLDERS` 从 app/workspace_session 移到 `roots`。回收站、文件剪贴板、在文件管理器中显示、新建/重命名 I/O 迁入基础设施；`context_menu::reveal_target` 改为 `workspace_actions::reveal_item`（git_menu、tab_context_menu 同步）。
- 与方案的偏差：`refresh.rs` 并入 `tree.rs`（规则不足以单列）；`model.rs` 推迟到 R5.2 与状态归属一起落地。
- 门禁：`features/explorer` 只允许 features/domain，禁止根状态/GUI 类型与例外；`explorer_filesystem` 只允许 features/domain/std::fs/std::process/windows_sys；新增 2 组负例。policy：file_tree、workspace_actions 增加 features/infrastructure 依赖，移除 workspace_actions 的 windows_sys 与 context_menu 的 std::process。
- 测试迁移：workspace_actions 中 8 项（MRU、根不重排最近目录、新建校验 2 项、重命名校验、Windows 设备名、Windows shell 路径）与 file_tree 中 3 项（排序、幂等刷新、剪枝）改为 feature 内存端口测试或基础设施测试，原名保留；file_tree 仅保留延迟刷新测试并增加 affected 路径断言。新增：lazy 目录不被刷新、缺失根保留与根外剪枝、affected 刷新、未变化保持 Arc 身份、已存在目标拒绝、hydrate 去重/规范化、最深根、Unicode 新建名、读失败为空、新建文件不覆盖。
- 结果：完整 workspace 测试 625 通过、0 失败、12 忽略（应用 541，编辑核心 65+3，独立域 16）；fmt、CI 参数 clippy、门禁与 20 项门禁自测通过；登记根字段 98、legacy exception 文件 60 不变。release、隔离启动与性能采样留到 R5 收尾统一执行。
