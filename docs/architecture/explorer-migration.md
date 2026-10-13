# 阶段 5：Explorer 迁移

2026-10-13，接续 R4 检查点 a29abb5，分支 `refactor/r5-explorer`。Explorer 的目录缓存、
根目录与最近目录、行内新建/重命名、右键菜单和树视图已从根状态与 `workspace_actions.rs`
拆出。执行方案与逐步记录见 [迁移方案](explorer-migration-plan.md)，验收见
[R5 报告](results/2026-10-13-r5-explorer/report.md)。

```text
src/features/explorer/          无 lgui、AppState、Workspace、InputState 或文件 I/O
  types.rs                      DirEntry、请求与右键目标
  ports.rs                      DirectoryPort / FileOpsPort
  tree.rs                       TreeCache：懒加载列表、展开、刷新、剪枝、可见行
  roots.rs                      根目录规范化、hydrate、最近目录 MRU、最深根匹配
  edit.rs                       新建/重命名名称校验
  model.rs                      ExplorerModel（字段私有）：根、最近、缓存、编辑请求、延迟刷新
  tests.rs                      内存端口测试
src/infrastructure/explorer_filesystem.rs
                                读目录、新建/重命名、回收站、文件剪贴板、在文件管理器中显示
src/ui/drawer.rs                DrawerMode、抽屉头部与通用图标（Explorer/Search 共享 chrome）
src/ui/explorer/
  state.rs                      ExplorerUiState（悬停、菜单、行内输入、滚动）与 ExplorerPanelState
  commands.rs                   语义 Command、只读 Frame、Git Decoration、EditInput
  view.rs                       文件树（含粘性目录、滚动条、空状态、缩放手柄）
  context_menu.rs               右键菜单
src/app/
  explorer.rs                   Explorer 命令与跨功能事务（一次根 State 更新）
  explorer_view.rs              投影组装、帧输入与命令执行
  explorer_tests.rs             命令在真实根状态上的测试
```

## 状态归属

AppState 删除 24 个字段，换成 `explorer: ExplorerPanelState { model, ui }` 与
`drawer: DrawerChrome`。根字段登记 98 → 76。`explorer_ownership` 逐字段登记模型 6 项与
UI 13 项，门禁要求结构体字段与登记一致、`ExplorerModel` 字段不得 `pub`。

抽屉的 visible、mode、width、resizing 由 Explorer 与 Search 共用，归 app chrome，而不是
最初登记的 Explorer 私有模型；这一归属更正在 policy 中逐项写明。

`workspace_folders` 在业务上是工作区根目录：Explorer 模型拥有它，Search、Git、终端、
项目设置和会话只经 app 取得 `roots()` 的只读切片或副本，不依赖 `features/explorer`。

## 事务与 I/O

重命名、删除、移除根、打开/拖放树文件、新建完成都会同时修改 Workspace 与 Explorer，
仍在一次根 State 更新中完成。模型方法只返回结果（是否需持久化、reveal 行号等），持久化与
回收由 app 处理。读盘与平台操作全部经 `ExplorerFilesystem`；视图不读盘，展开目录通过
`Command::ToggleDirectory` 由 app 同步读取，时序与迁移前相同。

watcher 的准备阶段只克隆 Explorer 模型。应用时以 `Arc` 身份拒收过期列表、编辑行打开时整批
延迟、根目录变化时整批丢弃；6 项竞态测试覆盖这些路径。对照测量显示与克隆整个根状态相比
没有可测差异，收益是结构上的。

## 视图接口

文件树与右键菜单接收 `StateAccess<ExplorerPanelState>`、`Frame`（活动文件、缩放状态、是否
可缩放、Git 装饰函数）与 `Commands`。本地交互（悬停、滚动、滚动条、菜单悬停/关闭）直接
写投影；涉及其他功能的操作——打开菜单时关闭标签菜单、文件拖动与点击计数、中键固定打开、
提交编辑、抽屉缩放、终端、剪贴板、磁盘对账——都作为命令交给 `app/explorer_view`。

## 门禁

```sh
node --test scripts/architecture/check.test.mjs
node scripts/architecture/check.mjs
```

- `features/explorer` 只依赖 features/domain，禁止根状态、GUI 与输入组件类型及例外。
- `infrastructure/explorer_filesystem` 只允许 std::fs、std::process 与 windows_sys。
- `ui/explorer` 禁止 AppState/AppRuntime/Workspace/ApplicationHandle 与例外；菜单宽度测量
  单独允许 unicode_width（与编辑器视图同例）。
- Explorer 字段逐项登记、模型字段私有、根状态不得 re-export Explorer 类型。

## 遗留与后续

- welcome、workspace_home、tab_context_menu 仍是登记的根状态适配器并调用 `crate::app`：
  剩余依赖是悬停、主表面焦点、打开文件、Source Control 开关与标签关闭等 chrome，随 R8 收窄。
- git_menu、git_panel 经 `app::explorer` 打开抽屉文件与复制路径，随 R6 Git 迁移。
- file_watcher、file_loading、file_icons、workspace_actions 的剩余部分不属于 Explorer，
  已按实际归属改登记到 R6/R8。
- 已知既有行为：重命名包含嵌套根的目录后，该根失去已加载内容与展开状态；每次刷新后对所有
  已缓存目录逐个 `is_dir` 的剪枝是单项变更的主要耗时。两者均早于 R5，未在本阶段修改。
