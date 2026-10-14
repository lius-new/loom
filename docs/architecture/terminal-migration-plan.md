# Loom R7 终端迁移方案

2026 年 10 月 14 日，基础提交 `b414223822337bd61b840950cd94826921e10a72`，起草时分支 `main`。R6 已完成；执行分支为 `refactor/r7-terminal`，R7.1–R7.5 已完成当前机器自动验收，R7.6–R7.7 尚未开始。

R7 将终端的纯规则、PTY 与系统适配、会话所有权、应用命令、effect 和视图分开。核心约束是移动终端时保留同一 PTY，快照不能保活进程，默认终端区域与分离区域保持各自的可见性，并兼容现有设置和会话。按七个检查点推进，每步保留可编译、可测试的实现。

## R7 开始前的实现

### 源码与测试规模

以下为当前源码总行数，包含空行和测试；测试数为源码中的 `#[test]` 声明，包含平台条件与 ignored 项，不能当作本机执行通过数。

| 来源 | 行数 | 测试声明 | 主要职责 |
| --- | ---: | ---: | --- |
| `src/terminal_session.rs` | 2587 | 27，其中 5 ignored | ShellKind、标签和分组、控制器、PTY、线程、ANSI 快照、选区与粘贴 |
| `src/terminal_directory.rs` | 516 | 5 | OSC 9;9 解析、cwd 回退、启动集成脚本及临时文件清理 |
| `src/terminal_mouse.rs` | 259 | 4 | 滚轮积累、scrollback 与 TUI 鼠标协议；目前依赖 lgui 修饰键 |
| `src/terminal_profiles.rs` | 320 | 3 | 配置 DTO、参数解析、shell 发现、可执行文件解析和默认回退 |
| `src/terminal_program.rs` | 433 | 5 | 稳定程序标签、进程树/前台程序查询、Windows console observer |
| `src/ui/terminal.rs` | 2260 | 12 | 绘制、输入字节、剪贴板、选区、标签、shell 菜单、分组和区域接线 |
| `src/ui/terminal_settings.rs` | 843 | 3 | 配置编辑表单、验证、增删、默认选择与路径 I/O |
| `src/ui/terminal_tab_context_menu.rs` | 401 | 1 | 目标终端菜单、用途、新建/分割/批量关闭及焦点恢复 |
| `src/model/application_layout.rs` | 1204 | 13 | 应用区域拓扑、尺寸、默认区域归属、移动计划与提交校验 |
| `src/ui/layout_map.rs` | 1514 | 8 | MapStore、命中几何、缓存预览、拖动提交、分隔条与布局缩略图 |

八个终端文件合计 7619 行、60 项测试声明；加上两个布局文件共 10337 行、81 项声明，其中 5 ignored。源码路径、hash、测试名称与起始位置另存于 `r7-terminal-inventory.json`。实施时还要冻结全 workspace 的可执行测试清单，不能只保留这 81 项。

主要外部接线在 `app/shell.rs`、`app/state.rs`、`app/workspace_session.rs`、`key_actions.rs`、`features/keymap/context_stack.rs`、`ui/settings.rs`、`ui/statusbar.rs` 和 `ui/workspace_home.rs`。构建脚本编译并嵌入 `scripts/terminal_console_probe.rs`；移动源码时需保留 observer 的构建、提取与启动链路。

### 会话和运行时

`TerminalTabs` 是 shell 中独立的 `State`，没有并入 AppState。其四个私有字段是 tabs、active_id、next_id、profiles。每个 `TerminalTab` 又持有 `TerminalController`；控制器通过 `Arc<TerminalInner>` 拥有 vt100 parser、PTY、writer、状态、选区、鼠标、generation、cwd、profile 和回退 notice。因此当前克隆标签视图也会保活运行时。

同组终端使用 group_id 与 width_weight；`region_view` 克隆相同控制器，只调整展示分组。`detach_group` 改归属，不重启终端。关闭会返回控制器，由调用方 terminate；终端内部最终 Drop 也清理进程。

Windows 使用 ConPTY，其他平台使用 portable-pty。`ensure_started` 只启动 Idle 会话，其他状态仅 resize；当前 shell effect 同步调用启动，输出解析和退出等待在后台线程运行。R7 保留启动调度，不顺带改为异步启动。输出缓冲为 8 KiB，正式会话 scrollback 为 2000 行，Windows waiter 每 50 ms 检查退出。

`restart` 增加 generation、终止旧进程并重置 parser/选区/鼠标。reader 和 waiter 通过 Weak 与 generation 检查识别旧运行。迁移要保留这些检查，并在新增投递边界校验会话身份；现有检查不等于已证明所有线程交错均安全，竞态测试若暴露旧缺陷须单列处理。

### effect 与持久化

shell 负责恢复终端、给控制器更新 profile、按可见区域启动和 resize、每 250 ms 探测程序及 cwd、合并通知、保存会话、光标闪烁和菜单关闭后的焦点恢复。程序候选持续满 1 秒才显示标签，退出立即清除；标签优先级是手工用途、运行程序、shell 名称。

OSC 9;9 解析跨 PTY read 保留状态，接受 BEL/ST 结束，拒绝控制字符、相对路径与异常报告，32 KiB 上限及恢复规则保留。ShellIntegration 用临时脚本加载用户 profile、发送目录报告，不改用户配置文件；显式 command/script/startup 参数优先。

`WorkspaceSession` 目前保存 terminal_tabs、terminal_groups、terminal_widths、terminal_purposes、terminal_directories、active_terminal、show_terminal 和 terminal_height。它不保存运行程序、PTY、屏幕内容或 application_layout 的完整区域拓扑。R7 不扩展会话格式，分离区域的重启行为按现有实现验收，不能宣称恢复完整拓扑。

### 状态登记与待更正项

当前门禁登记根字段 40 项、editor 字段 31 项、workspace 字段 9 项、遗留例外文件 41 个、根状态 UI 适配器 16 个。

根状态有 13 项登记 R7：application_layout、application_sash_drag、default_shell_fallback、resizing_terminal、selecting_terminal、shell_editor、show_terminal、terminal_focused、terminal_h、terminal_shell_menu、terminal_shell_scroll、terminal_split_drag、terminal_tab_context_menu。

另有六个配置字段登记 R8：default_shell、terminal_profiles、shell_options、terminal_cursor_blink、terminal_font、terminal_font_input。`editor.last_terminal_tab_click` 属于 R3 editor 交互状态。`application_layout.rs` 的文件例外却仍标为 R3 R4，与根字段的 R7 归属不一致；R7.1 按真实迁移归属逐项修正。

## 范围与行为约束

R7 覆盖八个终端文件、application_layout 纯规则、终端布局缩略图、相关应用命令和 effect。普通设置页、快捷键体系、其他 chrome、文件加载和 watcher 的整体迁移留给 R8；它们只调整终端类型路径与调用接线。

保留现有 GUI 布局、文字、输入协议、发现优先级、参数规则、轮询频率、进程探测和会话 JSON。保留 Cargo.toml、Cargo.lock、工具链、二进制名和现有 PTY/vt100 库；不增加新的 terminal crate。保持独立终端模型 State，不把会话列表并入 AppState，不增加文档副本或重复会话状态。根状态投影的 clone 成本仍归后续处理。

迁移必须满足以下行为约束：

1. 移动、切换、拖放和分离终端保持同一 PTY、cwd、屏幕、scrollback 与选区；split 创建独立进程并继承目标 shell 和最新 cwd。
2. 默认区域的 toggle 只控制默认区域；分离区域继续可见。新建终端加入默认区域，当前选中分离区域也不能改变默认归属。
3. 新建、split、保存和恢复使用当前有效 cwd，而不是可能晚一个 250 ms 探测周期的旧缓存。
4. 配置失效时保留原 shell 身份、profile 和参数；启动采用回退并显示 notice，修复后再次启动原 shell。删除默认自定义 profile 才重置默认选择。
5. 输入目标是渲染/操作时的 SessionId；旧组件不能把输入送到后来激活的另一终端，已关闭目标不执行命令。
6. Ctrl+C 有选区时复制，否则发送中断；复制失败保留选区且不发送中断。右键有选区复制，否则粘贴。换行归一化与 bracketed paste 字节保持一致。
7. 应用光标模式、xterm 修饰键、Shift 滚轮覆盖、TUI tracking、分数滚轮和宽字符位置保持一致；普通文字只由已提交的 input 路径发送，避免重复输入。
8. 菜单针对右键目标；Others/Right 的分组语义、批量关闭顺序、关闭最后会话和菜单关闭后的焦点按原行为保留。
9. 布局计划校验 version、viewport、visibility 和 sessions；真实释放与取消的判定、缓存预览提交及 5 px 拖动阈值保持一致。
10. MapStore 保持独立通知；同一区域内的无可见变化指针移动不通知，也不引起应用根组件重绘。

## 目标结构与所有权

```text
src/features/terminal/
  mod.rs types.rs              SessionId、ShellKind、状态、屏幕/单元格快照、无 GUI 输入类型
  model.rs                     TerminalModel：会话元数据、顺序、活动项、分组、宽度、用途
  screen.rs                    vt100 状态、run/颜色、选区和 scrollback 的纯运算
  input.rs mouse.rs            终端键/粘贴字节与鼠标协议；不接受 lgui 事件
  directory.rs                 OSC 9;9 流式解析
  profiles.rs                  ShellProfile DTO、参数编解码、给定候选下的默认回退
  program.rs                   给定进程信息的候选规则与 1 秒稳定标签规则
  ports.rs                     按 SessionId 的会话操作、快照与发现契约
src/domain/application_layout.rs
                               拓扑、尺寸和 DropPlan；使用 domain::Rect/Axis/Direction
src/infrastructure/terminal/
  mod.rs session.rs            SessionId → 运行时资源；parser 同步、generation、reader/waiter
  pty.rs                      平台启动、输入、resize、wait 和 terminate
  profiles.rs                 PATH/COMSPEC/SHELL/login shell/Git Bash 发现、可执行路径检查
  integration.rs              cwd 验证、ShellIntegration、临时脚本和环境
  program.rs                  原生进程查询、SessionConsole 与 observer
src/app/
  terminal.rs                 新建/选择/split/关闭、输入/剪贴板、profile 和跨功能命令
  terminal_effects.rs         恢复、可见会话启动、resize、探测通知、保存和清理
  terminal_view.rs            会话/区域/设置卡片的投影、Frame 与命令执行
  application_layout.rs      拓扑接线、拖动准备/提交、两容器更新与焦点结果
  state/layout.rs            应用布局 chrome
  state/terminal_map.rs       独立 MapStore、监听器和瞬态拖动状态
src/ui/terminal/
  state.rs commands.rs       TerminalUiState、ProfileEditor、命令、只读帧输入
  view.rs screen.rs tabs.rs   终端展示、坐标命中、标签与 shell 菜单
  keys.rs                    lgui KeyboardEvent → 无 GUI TerminalKey/Modifiers
  settings.rs menu.rs         shell 配置卡片与目标终端菜单
  layout_map.rs               缩略图/分隔条绘制、release hover 判断与命令意图
```

目录是职责目标，实际文件可合并，不能为了文件数拆碎函数。`features/terminal/screen` 可以使用 vt100，但没有 I/O、线程、运行时锁和 GUI；调用方的 parser 资源及其同步留在 infrastructure。端口只传 SessionId、值类型、请求、结果和快照，不暴露 PTY、reader、writer、controller、ApplicationHandle 或 State。使用端口/通知回调便于 fake runtime 与可控 UI 投递测试，不为生产执行额外增加线程或队列。

`TerminalModel` 继续使用单独的 State，只拥有元数据，不含 controller。`AppRuntime` 持有终端运行时服务，服务内部按 SessionId 管理资源；模型快照和区域帧不能延长进程生命周期。屏幕只有一份，UI 按需取得可见会话快照，不复制完整 scrollback 到 AppState。无 controller 的 metadata 仍需结合实时运行时 cwd 构造新建/保存请求。

app 组合 feature、UI 和 infrastructure；UI 仅依赖纯类型、StateAccess、帧输入和语义命令。基础设施使用 feature/domain 类型。application_layout 属于多个区域共用的 domain，不放进 terminal model。

### 字段归属

| 当前字段 | 目标 | 处理理由 |
| --- | --- | --- |
| application_layout、application_sash_drag | app 的 ApplicationLayoutState | 影响 Editor/Git/Files/Terminal，多区域共同布局 |
| show_terminal、terminal_h、resizing_terminal | app 的 TerminalChrome | 默认区域可见性和尺寸，不是 PTY 状态 |
| terminal_focused、selecting_terminal、terminal_split_drag | TerminalUiState | 输入路由和指针交互；焦点与 editor 联动由 app 执行 |
| terminal_shell_menu、terminal_shell_scroll、terminal_tab_context_menu、shell_editor | TerminalUiState | 菜单、列表滚动、目标与带 InputState 的表单 |
| default_shell_fallback | feature 的 TerminalAvailability，经 app 组合 | 给定配置和发现结果的纯回退状态，app 管理通知 |
| default_shell、terminal_profiles、shell_options、blink/font 相关六项 | 保持原 R8 配置归属 | R7 通过 app 提供只读配置帧和保存命令，不提前迁移设置体系 |
| editor.last_terminal_tab_click | TerminalUiState | 只在终端标签交互中使用；从 editor 逐项移出并登记理由 |

根中可用 `terminal: TerminalPanelState { availability, ui }`、`terminal_chrome` 和 `layout` 组合；其中不再放会话列表。逐字段登记实际落地值，不能用字段数量作为完成目标。

`ui/settings.rs` 在 R7 仍可保留 R8 根适配，但传给终端卡片的是窄 Binding 和由 app 计算的项目 shell 说明。`AppState.settings/apply/settings_at` 的 JSON 映射继续保留，必要的旧方法只做代理；终端 UI 不再执行路径 canonicalize、I/O 或直接修改设置根字段。

## 分步迁移

每步单独形成检查点提交。正式实施前冻结基础源码、全 workspace 执行清单、ignored 用途、当前机器工具链和性能样本。编译产物与采样二进制记录 SHA-256。当前安装版 Loom 不作为本轮验收目标，验收只运行本轮构建或打包的二进制。

### R7.1 提取纯类型和规则

迁移 ShellKind/DTO、颜色与 run、选区、粘贴、OSC、参数、给定候选的 fallback、稳定程序规则。将 `key_bytes` 的 GUI 转换与协议编码分开，用简单 Modifiers 去掉 terminal_mouse 对 lgui 的依赖。不将发现、spawn、临时脚本或 OS probe 一并放入 feature。

将 application_layout 迁入 domain，UiRect 改为现有 Rect；UI/app 在边界转换坐标，Axis/Direction 直接使用纯域路径。保持浮点算法、最小尺寸和计划校验，不保留旧 model 路径别名。逐项更正该文件 R3 R4 例外与 R7 归属。

验收：纯规则测试按原名迁移；无 GUI 域验证覆盖新 layout 模块（测试迁移映射另记，新增的独立重复验证单列）。新增门禁拒绝 GUI、I/O、运行时、源码包含和例外。完整应用回归和原生测试仍需可用，不能用纯测试替代它们。

### R7.2 迁移 PTY 和系统适配

将平台启动/resize/wait/kill、可执行文件发现、ShellIntegration、原生进程探测、SessionConsole 及现有控制器调度迁入 infrastructure/terminal。先保持控制器行为与线程结构，随后 R7.3 才移除模型对资源的引用。

通过 app 注入帧通知/事件接收回调；infrastructure 不引用根状态和 app，不直接持有 GUI 调度句柄。保留 Windows 参数 quoting、环境合并、隐藏 observer、startup 参数优先、临时文件生命周期、Unix shell/PTY 路径与失败文案。

验收：真实 shell/cwd/鼠标及四项 opt-in Windows smoke；测试辅助 `native_mouse_probe` 和精确入口随模块路径同步。验证 fallback 修复与 close/drop，检查 observer 提取和 build.rs 产物仍有效。迁移旧顶层适配后删除原路径，例外只针对尚未迁走的部分保留。

### R7.3 分离会话模型和运行时所有权

用纯 TerminalModel 接管会话元数据、active、group、width、purpose、顺序和关闭后邻居选择。SessionId 保持现有 u64 身份分配和重启恢复语义；profile 源在 R8 设置中，按 app 的配置帧提供，不建立第二份配置事实源。

运行时按 SessionId 持有资源，安装到 AppRuntime。新建分配资源，移动只改归属，split 创建新资源。close 先断开模型/布局引用，再从运行时移除并终止目标，效果不在 State 锁内执行。状态快照不可克隆 controller，应用 teardown 显式清理，不依赖 UI 回调或快照 Arc 数量。

验收：fake runtime 检查每个 SessionId 的 create/terminate 次数；模型和区域快照不保活服务；移动不 spawn/terminate，split 恰好新增一个，重复 close/shutdown 幂等。原有移动 leader、cwd、scrollback、选区与相邻关闭测试保留。逐项登记模型私有字段，拒绝运行时类型回嵌。

### R7.4 收拢状态和应用命令

按字段表迁移 13 项根字段及 editor 的终端点击状态。把 UI 中的新建/选择/split/关闭、批量关闭、用途、toggle、profile 保存/删除/默认选择、clipboard 与输入执行放到 app/terminal。纯 close_targets 进入 feature；文件检查、profile ID 的时间/顺序来源由 app/适配器提供。

命令带 SessionId/菜单目标，app 执行前重查目标。跨 terminal model 与根 layout 的修改由同一个 UI 回合协调；它们仍是两个容器，不能称为跨 store 原子事务。保持既有更新顺序，并增加完成回合后的归属不变量检查，禁止引入嵌套锁顺序反转。

验收：已关闭目标、非活动菜单目标、最后一个会话、默认区域 hide 与 detached focus；Copy/Paste 失败时的选区和字节；profile 表单 blur/unmount 保留状态、参数不展开、项目 default_shell 覆盖保持。context_stack、statusbar、workspace_home、settings 只改为 app 命令/投影调用，仍登记其余 R8 依赖。

### R7.5 迁移 effect 和会话协调

shell 留下 effect 安装入口，将恢复、profile 更新、可见区域的 ensure_started/resize、程序/cwd 探测、终端保存和清理迁入 app/terminal_effects。时钟和 UI 投递可注入，复用现有 ManualClock/UiTasks；保留 250 ms 探测和 1 秒标签规则。

通知带 SessionId 与运行代次，已关闭、旧运行或旧 effect 的通知不得改写新目标；旧 effect 清理只停止自己。每次启动/resize 只覆盖实际可见的区域，hide 不 terminate，输出不把全部屏幕写入 reactive model。保存前取得当前运行时 cwd，保留正常退出及含脏文件退出流程中既有保存时机。

验收：旧 reader/exit/失败、旧 probe/cwd 通知晚到，close 后投递，restart 后旧结果，effect 重装后的旧清理；稳定程序标签覆盖 999/1000 ms，探测周期覆盖 249/250 ms。并发测试使用可控 fake 交付，不靠增加 sleep 偶然通过。

会话 JSON 的八个终端字段和 ShellKind/Custom(u64) serde 完全兼容；旧/缺字段、无终端、非法 active index、宽度归一化、用途迁移和 cwd 回退均测试。运行程序和运行时资源不持久化，区域拓扑继续不新增持久化。

### R7.6 迁移终端视图和布局缩略图

可拆为 R7.6a 终端屏幕/标签/菜单/配置卡片、R7.6b layout_map/分隔条。各视图只接收窄 StateAccess、只读会话/屏幕/区域帧、焦点能力和语义命令；不接收 AppState、TerminalController、AppRuntime 或 infrastructure。

MapStore 和通知订阅留在 app 的独立瞬态对象，UI 得到只读 Observable/帧与 Pointer/Release/Cancel/Resize 等命令。MapScene 几何准备、布局提交和 settle 由 app 编排；真实 hover 判断留在 UI，传入 release 事实供 app 决策。源模型的纯 plan/commit 保留在 domain。

设置输入改用 InputBinding::new_access/render_access，保留 InputState/IME/焦点；UI 的 key adapter 保留在 ui/terminal/keys。屏幕字体、颜色、宽字符 clip、焦点光标与 native IME cursor rect 的绘制保持原样。

验收：迁移完整 UiSession 事件测试，保持测试名、事件步骤与断言。重点检查 inactive pane 输入、modal 阻断、菜单/配置关闭后焦点、5 px threshold、cancel PointerUp、cached preview、T junction、窗口收缩、不可用落点和 zone 内零通知。删除四个旧 UI 模块及 ui/mod 声明，不保留 re-export 别名。

### R7.7 门禁和验收收尾

清理五个旧终端顶层模块、三份旧终端 UI、旧 model/application_layout 和 ui/layout_map 路径、对应例外/根适配器/runtime 桥接。核对 13 项旧 R7 字段均已迁移或有确切保留原因，关闭 application_layout 的过期例外；不整体刷新 policy。

同步 baseline 精确过滤器、build 路径、所有 cfg 测试入口和性能过滤器；旧测试名称一对一映射到新完整路径。逐项检查过滤器实际匹配数，exit 0 且运行 0 项不能算通过。`native_mouse_probe` 用于测试子进程，禁止全局执行所有 ignored 项。

完成后写 `terminal-migration.md`、`results/<date>-r7-terminal/report.md`、validation/test-migration/performance 原始记录并更新 progress。报告单列新增行为测试、独立域重复验证、ignored 与重跑，记录仍待验项目。R8 不在本轮启动。

## 验收清单

### 自动行为矩阵

| 项目 | 保留或新增覆盖 | 合格条件 |
| --- | --- | --- |
| shell 配置与回退 | custom_profiles…、unavailable_custom_default…、profiles_can_be… | 参数、身份、失效配置、恢复后启动及删除默认语义一致 |
| cwd 与集成 | real_shell_directories…、reports_survive…、custom_startup_commands… | cd 报告、read 边界、显式启动参数优先、新建/split/保存读最新 cwd |
| ANSI 与文字 | ansi_parser…、wide_terminal_cells…、selection_text… | 色彩、宽字符、换行、wrapped rows、scrollback 选区与光标一致 |
| 输入与剪贴板 | modified_navigation_keys…、terminal_keeps_shell_keys…、新增失败注入 | Ctrl+C/右键、bracketed paste、修饰键、IME 提交和目标路由一致 |
| 滚轮与 TUI | application_wheel…、fractional_wheels…、conpty_delivers_wheels… | 原生报告、Shift override、历史恢复、坐标上限无回归 |
| 分组和区域 | splitting_keeps…、moving_a_group_leader…、detached_sessions… | 同一资源移动、split 独立、默认与分离区域互不影响 |
| 拖放与缩略图 | 现有 13 项 layout + 8 项 map 测试 | 预览无副作用、过期计划拒收、取消不提交、独立通知不退化 |
| 菜单/表单/焦点 | tab_context_menu…、profile_inputs…、真实 pointer capture | 目标重查、modal 不送 shell、退出菜单/表单后继续输入 |
| 所有权与竞态 | fake PTY/时钟/UiTasks 的新增测试 | 快照无保活、关闭/重启旧结果拒收、cleanup 幂等、不创建额外 PTY |
| 会话和退出 | 旧 JSON 夹具、完整 workspace 回归、隔离启动/恢复 | 字段/顺序/分组/宽度/cwd 兼容，未保存文档退出流程不受影响 |

表中省略名用于阅读，完整名称以 inventory 和正式执行时冻结的清单为准。不能只按数量比较，必须逐项映射旧路径、新路径、平台、ignored 和结果。

### 每个检查点的工程检查

每个实现检查点要求架构门禁及自测、报告自测、fmt、CI 参数 Clippy、debug 完整 workspace 测试、release 构建和隔离启动通过。R7.2、R7.3、R7.5 及最终收尾另跑真实终端专项；最终跑默认环境 debug、独立 Git 配置 debug、release 完整回归、四项 Windows smoke 与无参数恢复。保持其他功能全部测试，不只运行 terminal filter。

以下是执行时的命令，不是本方案已完成的验收。Windows 使用 PowerShell 7；构建 helper 在存在匹配 Skia 缓存时使用已校验归档。

```powershell
node --test scripts/architecture/check.test.mjs
node --test scripts/baseline/test-results.test.mjs
node scripts/architecture/check.mjs
cargo fmt --all -- --check
./scripts/build/cargo.ps1 clippy --locked --offline --workspace --all-targets -- -D warnings -A clippy::redundant_closure_call -A clippy::collapsible_if -A clippy::too_many_arguments -A clippy::let_and_return -A clippy::manual_div_ceil
./scripts/build/cargo.ps1 test --locked --offline --workspace
./scripts/build/cargo.ps1 test --release --locked --offline --workspace
./scripts/build/cargo.ps1 build --release --locked --offline --bin Loom
./scripts/baseline/run.ps1 -Offline -PerformanceSet None -OutputDirectory target/baseline/<fresh-r7-checkpoint>
./scripts/baseline/startup-smoke.ps1 -OutputDirectory target/baseline/<fresh-r7-startup>
```

baseline/run 包含部分重复检查，实际执行可按记录合并，但不得漏掉独立配置、release 和 smoke。每次使用新的结果目录；完整 test --list 与 hash 在执行前后保存。Rust 回归基准是 R6 仓库报告的 673 通过、14 ignored，其中独立域重复验证 16；必须先在当前机器实测建立 R7 起点，不能把另一机器的报告直接写成当前通过数。

四个 Windows opt-in 入口是 supported_shells_round_trip_through_real_conpty_sessions、launcher_backed_shell_is_idle_then_detects_and_clears_programs、exited_shell_launchers_do_not_hide_unknown_applications、running_application_is_detected_and_cleared_in_each_real_shell。完整模块路径迁移后以 --list 校验，再用 --exact --ignored --nocapture --test-threads=1 单项运行。原有非 ignored 的 ConPTY wheel 测试仍参加全量回归，首次失败和重跑都保留。

### 新增门禁

- features/terminal 与 domain/application_layout 禁止 GUI、根状态、进程/文件/线程和迁移例外；允许明确登记的 vt100/serde 与纯类型依赖。
- ui/terminal 禁止 AppState、Workspace、控制器、运行时、app/infrastructure 直接调用、路径别名、源码包含和迁移例外。
- infrastructure/terminal 不反向依赖 app/ui/root，平台权限限定为终端所需的 I/O、线程与系统 API；UI 调度由 app 注入。
- 根、终端模型、UI/chrome/layout 的字段逐项登记；纯模型字段私有；快照及模型不得包含运行时服务、PTY、writer 或控制器。
- 负例覆盖新增和过期字段、嵌套资源、分组 import、重导出和 include 绕过；检查 `main.rs`、`ui/mod.rs`、baseline 和辅助 probe 的残留旧路径。

### 性能对照

保留常规 31 项基线，在同一机器/工具链建立 before/after 数据。现有 `terminal_parse` 使用 8 MiB ANSI/中文流；`terminal_screen_snapshot_100` 实际测的是 `screen.contents()` 100 次，不是完整控制器快照、UI 绘制或原生帧延迟。其名称和历史测量保持不变。

另加终端专项：真实生产屏幕快照构造、终端视图帧构建，以及 1/8/32 会话下纯模型/帧投影。布局通知按现有测试断言检查，不能用耗时掩盖额外重绘。固定输入、字体、窗口和可见区域，before/after 使用冻结的 release 测试二进制，注入同一基准入口，每版预热 1 轮、正式至少 10 轮，交错采样、无并行构建。

所有样本、median、散布、节点/通知数、二进制 hash 与源码检查点保留。中位数持续上升超过 10% 时查明原因并复验；接近计时分辨率的百分比不作为改进结论。原生窗口输入延迟、内存和后台 CPU 如另测则单列，不能从 UI session 推断。发现性能问题须单列修复，不为过门禁调整输入规模或虚拟化算法。

### 原生与跨平台验收

| 场景 | 预期 |
| --- | --- |
| PowerShell/Cmd/Git Bash，中文/空格路径 | 启动、交互、cd、继承目录和重启一致 |
| 自定义 shell、显式 startup 参数 | 参数不展开，profile 保留，失效提示与修复恢复正确 |
| 中文 IME、Emoji、全角、组合字符 | 提交只输入一次，候选窗口定位与宽字符光标无偏移 |
| Ctrl+C、右键、失败剪贴板、多行粘贴 | 复制/中断边界、选区保留、bracketed 字节一致 |
| 普通历史与支持鼠标报告的 TUI | 滚轮/Shift 覆盖/退出 TUI 后历史恢复一致 |
| 新建、split、移动、区域 tab、批量关闭 | 资源不串联，移动不重启，关闭正确目标 |
| 默认区域 toggle、关闭最后默认 tab | 分离区域保持可见和可用，焦点不错误跳转 |
| 布局缩略图、边缘/分隔条/无效落点、窗口缩小 | 预览与提交一致，取消及旧预览不改变拓扑 |
| 菜单/设置表单/Escape/切换系统窗口 | modal 隔离，拖动结束，回到正确终端继续输入 |
| 正常退出、Save/Don't Save、恢复与应用清理 | 会话格式兼容、当前 cwd 保存、无本次创建的遗留 observer/PTY |
| Linux/macOS 默认 shell、profile 与 PTY | 启动、resize、cwd、输入、退出和前台程序探测实际通过 |
| Windows staged/portable/installer | 本轮二进制、嵌入 observer 和运行时资源完整，打包及安装验收结果单列 |

人工验收记录环境、构建 hash、步骤、预期、实际和时间；自动测试不能代替这些项目。只关闭验收创建的 PID，保留用户已运行的 Loom 和终端，不修改真实 APPDATA/LOCALAPPDATA 会话或用户 shell/Git 配置。远端 CI、真实安装/卸载及缺少的跨平台环境不能提前标为通过。

## 检查点完成标准

R7 自动收尾要求：各步工程门禁通过；所有旧执行测试完成路径映射且无丢失、无未解释失败；新增生命周期/跨容器/通知竞态覆盖通过；终端三层无迁移例外；旧 R7 字段、UI 根适配器和资源桥接关闭；JSON 兼容、冻结性能 A/B、隔离启动/恢复及终端原生 smoke 有可复验记录。

原生人工、其他平台、远端 CI 和真实安装若未完成，报告写为“R7 本机自动验收完成”，保留待验清单。不能只凭字段减少、目录变化或本机绿色测试宣布全部验收完成。

## 实施前方案检查记录

本次仅保存方案和源码盘点，未迁移生产代码。当前机器复跑的架构门禁通过，29 项架构自测与 3 项报告自测均通过。完整 Rust 回归、构建、性能、原生交互及打包验收留待 R7 实施；R7.1–R7.7 的执行记录尚未开始。

## R7.1 执行记录

2026-10-14：纯类型、屏幕/输入/滚轮、OSC、参数与候选规则迁入 features/terminal；应用布局迁入 domain/application_layout，GUI 坐标在边界转换，无旧路径别名。PTY、启动/发现、运行时与旧 UI 保留后续阶段。本机完整回归 688 通过、14 忽略，34 项原测试迁移无缺失；新增行为 2 项与独立布局重复验证 13 项分列。门禁及 36 项相关自测、fmt、严格 Clippy、release、四项 Windows 原生 smoke、隔离启动通过。同机修改前性能基线 31 项、155 条正式记录已冻结。实际 Rust 1.98.0，固定 1.97.1 及原生人工/跨平台仍待验。详见 [说明](terminal-migration.md) 和 [验收](results/2026-10-14-r7-1/report.md)。停在 R7.1，不推进 R7.2。

## R7.2 执行记录

2026-10-14：PTY、控制器调度、shell 发现、cwd 集成及原生程序探测迁入 infrastructure/terminal；app 注入 FrameNotifier 与 Palette。删除三个旧顶层适配路径，原控制器线程与 TerminalTabs 所有权保持原样。完整 workspace 689 通过、14 忽略；4 项源码测试迁移、605 个旧应用执行入口无缺失。新增回调/generation 测试并补强自定义 shell 修复后真实重启验收。架构/报告自测共 38 项、门禁、fmt、严格 Clippy、release、四项 Windows smoke、隔离启动及 observer 提取校验通过。终端基础设施无例外，遗留例外从 39 减至 36。当前 Rust 1.98.0，固定 1.97.1、原生人工/跨平台及完整性能 A/B 仍待验。详见 [验收](results/2026-10-14-r7-2/report.md)。停在 R7.2，R7.3 未开始。

## R7.3 执行记录

2026-10-14：TerminalModel/TerminalMetadata 迁入纯 feature，10 项字段私有登记；独立 State 只持纯模型。AppRuntime 持有按 SessionId 管理的运行时资源；app 临时绑定和句柄均为 Weak。移动只改元数据/布局，split 新增资源，实时 cwd 在状态锁外取得；先断开模型/布局引用再释放注册资源。关闭身份不能重建，重复 close/shutdown 幂等，dispose 与启动协调且通知可重入关闭。Profile 通过当前配置 getter 注入，模型不保存副本。旧 terminal_session 删除；5 项纯测试与 14 项原生/跨层测试保留原名迁移，精确 native smoke 与辅助入口同步到 app::terminal_session_tests::tests。完整回归 696 通过、14 忽略，606 项旧应用入口无缺失，新增行为 7 项。真实布局提交保持 PID/generation，持有快照时通过 Windows 句柄确认 shell/observer 实际退出。42 项自测、架构门禁、fmt、严格 Clippy、release、四项原生 smoke、隔离启动及 36 项算法对照通过。实际 Rust 1.98.0；固定 1.97.1、原生人工/跨平台及完整性能 A/B 保留待验。详见 [验收](results/2026-10-14-r7-3/report.md)。停在 R7.3，R7.4 未开始。

## R7.4 执行记录

2026-10-14：13 项根字段与 editor 终端双击状态迁入明确所有者；新建/选择/split/关闭/用途/toggle、输入/剪贴板/profile 操作迁入 app，按 SessionId 和菜单目标执行前重查；close_targets 纯规则迁入 feature。模型、布局、清理顺序协调，完成 UI 回合检查归属，不称跨 store 原子事务。设置卡片改为窄投影/命令，六项配置字段保持 R8，草稿保留到 blur 完成。新增 6 行为测试，702 项通过、14 忽略，613 旧应用入口无缺失；46 自测、架构、fmt、Clippy、release、四项原生 smoke、隔离启动和 21 源码对照通过。根/editor/workspace 30/30/9，22 项终端归属，34 遗留例外，15 根 UI 适配器。详见 [验收](results/2026-10-14-r7-4/report.md)。R7.5 未开始，原有 shell effect 仅更新字段路径；固定 1.97.1、原生人工/跨平台及整体性能 A/B 保留待验。

R7.5 本机自动验收：effect/恢复/启动/探测/保存/清理迁入 app/terminal_effects，输出/退出生命周期锁内核对代次，UI 通知按 SessionId/代次/effect lease 拒收旧结果，旧 cleanup 不停止替换 mount。710 通过、14 忽略，新增 8 项，619 旧应用入口无缺失；47 自测、架构、fmt、Clippy、release、四原生 smoke、隔离启动通过。实际 Rust 1.98.0；固定工具链、原生人工/跨平台和最终性能验收保留。详见 [R7.5 验收](results/2026-10-14-r7-5/report.md)。继续 R7.6，R8 未开始。
