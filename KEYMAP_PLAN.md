# Loom Keymap 设计方案与实施规划

> 状态：M0–M7 已实现（手动回归待验证）· 2026-10-07
> 参考：Zed `crates/gpui/src/keymap*`、`platform/keystroke.rs`、`key_dispatch.rs`、`crates/settings/src/keymap_file.rs`、`docs/src/key-bindings.md`（main 分支）
>
> 完成标记：`[x]` 已完成 · `[ ]` 未开始 · `[~]` 进行中

---

## 0. 目标与非目标

**目标**

1. 快捷键与功能解耦：所有快捷键行为都表示为带命名空间的 **Action**（如 `editor::Undo`），按键只是指向 Action 的绑定。
2. 采用 Zed 风格的 `keymap.json`：按 **context** 分组、`null` 解绑、后定义者优先、用户配置覆盖默认配置。
3. **上下文感知**：同一个按键在 Editor / Terminal / Dialog 中可以有不同含义，由上下文树深度决定优先级，不再依赖各处 `stop_propagation` 的先后顺序。
4. 支持多键序列（`secondary-k secondary-s`），带等待超时、回放和状态栏提示。
5. 用户配置热重载、部分失败容错、错误提示。
6. 菜单和提示上的快捷键文字从 keymap 反查生成，保证“显示即实际”。

**非目标（本期不做）**

- 命令面板、可视化 Keymap 编辑器（但数据模型为它们预留）
- 预置键位方案（VSCode / JetBrains 的 base keymap），Vim 模式
- 非 QWERTY 键盘的 key equivalents 重映射
- 插件或扩展注册 Action

---

## 1. Zed 的设计拆解

### 1.1 核心数据模型

| 概念 | Zed 实现 | 要点 |
|---|---|---|
| `Keystroke` | `{ modifiers, key, key_char }` | `key` 是键帽字符（`alt-s` 的 key 是 `s`），`key_char` 是实际会输入的字符（`ß`）；匹配时两者都尝试 |
| `KeyBinding` | `{ action, keystrokes: SmallVec, context_predicate, meta, action_input }` | `meta` 记录来源（User / Base / Default），用于 `null` 的作用范围和 UI 展示 |
| `Keymap` | `Vec<KeyBinding>` + `action → indices` 索引 + 禁用绑定索引 + `version` | 插入顺序就是优先级的一部分；`version` 递增，供缓存失效使用 |
| `KeyContext` | `Vec<ContextEntry { key, value: Option }>` | 一个节点可以有多个标识和属性，如 `Editor mode=full extension=rs` |
| `KeyBindingContextPredicate` | `Identifier / Equal / NotEqual / Not / And / Or / Descendant(>)` | 语法：`Editor && mode == full`、`!Terminal`、`Workspace > Editor` |
| `Action` | `actions!(editor, [Undo])` 宏生成，带名字和反序列化 | JSON 中写法：`"name"` / `["name", arg]` / `["name", {..}]` / `null` |

### 1.2 Keystroke 的语法与规范化

- 写法：修饰键 `ctrl- alt- shift- cmd-/win-/super- fn-`，加上 **`secondary-`**（macOS 上是 cmd，其他平台是 ctrl）。
- 按键名：单个 Unicode 字符，或命名键（`tab`、`f1`、`escape`、`pageup` 等）。
- **shift 规则**：`shift-` 只和字母搭配，表示大写（`shift-g` 匹配输入 `G`）。标点由 shift 产生时**不算修饰**，所以 `shift-(` 不匹配，应该写成 `(`。
- 序列：多个按键用空格分隔，`"cmd-k cmd-s"`。
- Windows 特例：当 `key_char` 与 `key` 不同（AltGr 产生的字符），按“无修饰 + key_char”再匹配一次（见 `should_match`）。

### 1.3 上下文树与谓词语义

渲染时，每个元素用 `.key_context("Editor")` 声明上下文。从根到焦点元素的路径组成 **context stack**：

```
Workspace os=windows
  Pane
    Editor mode=full extension=md
```

`predicate.depth_of(stack)` 从最深层往上找，返回第一个使谓词成立的深度。这个深度就是该绑定的优先级。

- `Identifier` / `Equal` / `NotEqual`：只看**当前这一层**的节点；属性不会继承。
- `Not(X)`（v0.197 之后）：路径上**任何一层**都不满足 X 才成立。`!Editor` 在 `Workspace > Pane > Editor` 中为假。
- `A > B`：某个祖先满足 A，并且后续的子路径满足 B（“祖先”而不是“直接父节点”）。
- 没有 context 的绑定，深度视为最深层（`contexts.len()`），即全局生效且优先级最高。

### 1.4 优先级与解绑（`Keymap::bindings_for_input`）

1. 遍历所有绑定（逆序，后加入的在前），过滤掉上下文不匹配的，按键匹配分为“完全匹配”和“前缀匹配（pending）”。
2. 完全匹配的结果按 **(深度降序, 插入顺序降序)** 排序：深层优先，同一层后定义的优先。
3. `null`（`NoAction`）：会屏蔽排在它后面、**来源同级或更弱**的绑定。例如 base keymap 里的 null 能屏蔽 default，但屏蔽不了 user。
4. `Unbind`（`unbind` 段）：只移除“同按键 + 指定 action”的那一条，其他绑定不受影响。
5. pending 判定：如果存在更长的、以当前输入为前缀的绑定，并且它的优先级不低于已完全匹配的绑定，就进入 pending。
6. 返回的是**多个**绑定的列表，而不是一个：分发时依次尝试。前一个 Action 如果没有任何处理器接收（处理器调用了 `propagate`），就继续尝试下一个。这就是 Zed 文档里“条件 Action 回落到默认绑定”的机制。

### 1.5 分发流程（`DispatchTree::dispatch_key`）

```
input = pending + [keystroke]
(bindings, pending?) = keymap.bindings_for_input(input, context_stack)
if pending?            → 保存 pending，等待下一个键（如果同时有完全匹配，超时后执行它）
elif bindings 非空     → 按优先级依次 dispatch_action，沿焦点路径冒泡，遇到第一个处理者即停止
elif input.len()==1    → 未绑定：交给元素自己的 key 监听器，以及文本输入（IME）
else                   → replay：把旧的 pending 中能匹配的最长前缀作为一次执行，
                         剩下的递归重新分发；完全匹配不到的键作为普通输入回放
超时（约 1s）         → flush_dispatch：pending 中能执行的执行，其余回放为文本
```

Action 的处理器在渲染时通过 `.on_action(...)` 挂到元素上（每帧重建分发树），按焦点路径从深到浅冒泡。

### 1.6 Keymap 文件加载（`KeymapFile`）

- 文件格式是 JSONC（允许注释和尾逗号）。结构为 `[{ context?, use_key_equivalents?, unbind?, bindings? }]`。
- 使用宽松类型反序列化，**单条出错不影响其他绑定**。加载结果分三种：`Success` / `SomeFailedToLoad { 错误消息 }` / `JsonParseFailure`。最后一种保留旧的 keymap。
- 加载顺序：Default → Base → (Vim) → User。每条绑定通过 `meta` 标记来源，保证 user 最后加载、优先级最高。
- 默认 keymap 按平台各有一份：`assets/keymaps/default-{macos,windows,linux}.json`。
- 配套的调试工具：`dev::OpenKeyContextView` 实时显示当前 context stack 和最近一次按键匹配的结果。

### 1.7 显示

- `bindings_for_action(action)` 按插入顺序返回该 Action 的所有绑定，**最后一个用于显示**，并会过滤掉被 null 或 Unbind 屏蔽的。
- `highest_precedence_binding_for_action_in(context)`：在给定上下文中，返回真正会触发该 Action 的绑定，用于菜单和 tooltip。

---

## 2. Loom 现状与约束

| 位置 | 内容 |
|---|---|
| `src/input/keymap.rs` | `Action` 枚举加上写死的 `action_for`，在 root 的 `on_key_down`（冒泡阶段）处理 |
| `src/editor/commands.rs` | `Command` 枚举加 `command_for`（方向键、编辑、剪贴板），在编辑器的 `on_key_down` 中处理并 `stop_propagation` |
| `src/editor/editor_view.rs:1201` | `is_save_shortcut`（Ctrl/Cmd+S） |
| `src/ui/terminal.rs` | `clipboard_shortcut_for`（在 root 捕获阶段处理）；`key_bytes` 把 Ctrl+字母等透传给 pty |
| `src/ui/close_confirmation.rs`、`src/ui/clone_repository.rs` | 对话框各自的 Enter / Esc / Ctrl+D 处理，并拦截所有按键 |
| `src/ui/components/input.rs` | 输入框的文本编辑按键 |
| `src/editor/edit_menu.rs:17` | 写死的 `"Ctrl+Z"` 等显示文字 |

**lgui 约束**

- 没有 Zed 那样的 `key_context()` / `on_action()` 元素 API。焦点状态由 `AppState` 中的标志位表达（`focused`、`terminal_focused`、`clone_input_focused`、`close_request` 等）。
- root 上已有 `on_event_capture(UiEventKind::KeyDown, ..)`（`src/app.rs:458`），在捕获阶段调用 `stop_propagation` 有效（终端的剪贴板处理已经在用）。
- UI 是声明式的，每帧重新 render，因此可以仿照 Zed，**每帧重建 Action 处理器表**。
- 用户配置目录：`workspace_persistence::config_path("keymap.json")`，Windows 上是 `%APPDATA%\Loom\keymap.json`。

---

## 3. 设计方案

### 3.1 模块布局

```
src/input/
  mod.rs
  keystroke.rs     Keystroke / Modifiers：解析、规范化、从 KeyboardEvent 转换、显示文字
  context.rs       KeyContext、ContextPredicate（解析 + depth_of），移植 Zed 的语义
  action.rs        Action 枚举 + 名字表 + 参数解析 + 元数据（文档、是否有参数）
  keymap.rs        Binding / BindingSource / Keymap：bindings_for_input、binding_for_action
  keymap_file.rs   JSONC 预处理、KeymapFile 反序列化、部分失败加载、默认配置与用户配置合并
  dispatcher.rs    KeyDispatcher：pending 状态机、处理器注册表、分发与回放
  context_stack.rs fn context_stack(&AppState) -> Vec<KeyContext>
assets/keymaps/
  default.json     默认键位（用 secondary- 适配平台；平台差异用 os == xxx 的 context 表达）
```

`keymap.rs` 原有的 `Action` 和 `action_for` 将被替换；`editor::commands::Command` 保留为编辑器内部的执行原语，不再负责按键解析。

### 3.2 Keystroke

```rust
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct Modifiers { pub ctrl: bool, pub alt: bool, pub shift: bool, pub platform: bool }

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Keystroke {
    pub modifiers: Modifiers,
    /// 规范化后的按键名：小写字母、单个字符，或命名键（"tab"、"pageup"、"f5" 等）
    pub key: String,
    /// 实际会输入的字符（来自 LogicalKey::Character），用于 AltGr 和回放
    pub key_char: Option<String>,
}

impl Keystroke {
    pub fn parse(source: &str) -> Result<Self, KeystrokeError>;      // "secondary-shift-g"
    pub fn from_event(ev: &KeyboardEvent) -> Option<Self>;            // 纯修饰键按下时返回 None
    pub fn should_match(&self, target: &Keystroke) -> bool;           // typed vs binding
    pub fn display(&self) -> String;                                  // "Ctrl+Shift+G" / "⌘⇧G"
}
```

**规范化规则（`from_event`）**

| 输入 | 结果 |
|---|---|
| `Character("B")` + shift | `key="b"`，`shift=true` |
| `Character("(")` + shift | `key="("`，`shift=false`（标点自带 shift，与 Zed 一致） |
| `Named(Tab)` + shift | `key="tab"`，`shift=true` |
| meta 键 | `platform=true`；解析时 `cmd`/`win`/`super` 都映射到 `platform` |
| Windows 上 ctrl+alt 并产生非字母数字字符（AltGr） | 先按完整修饰键匹配；失败后按“无修饰 + key_char”再匹配；仍失败则放行为文本输入 |

**`secondary-`** 在解析阶段展开：macOS 上为 `platform=true`，其他平台为 `ctrl=true`。现有代码对 Ctrl 和 Meta 都接受，迁移后改为按平台各自生效，这是有意的行为收敛，会写入变更说明。

### 3.3 KeyContext 与谓词

直接移植 Zed 的 `KeyContext`、`ContextEntry`、`KeyBindingContextPredicate`，包括解析器和 `depth_of` / `eval_inner` 的语义（`Not` 看整条路径、`>` 表示祖先），以及 `is_superset`（用于 null 的作用范围判断）。约 400 行，Zed 的测试用例可以照搬。

**Loom 的上下文树**（由 `context_stack(&AppState)` 从状态推导，每次按键时计算一次）：

```
Workspace os=windows surface=editor|diff|settings|home|welcome
├─ Editor extension=rs mode=full [has_selection]   ← app.focused 且主区域为编辑器
├─ DiffEditor                                       ← 焦点在 diff 视图
├─ Terminal                                         ← app.terminal_focused
├─ Explorer [renaming]                              ← 后续补充：需要新增文件树焦点标志
└─ Menu                                             ← editor.menu / context_menu / tab_context_menu 打开时，作为最深层

Dialog CloseConfirmation                            ← 模态：独立的根
Dialog CloneRepository > Input                      ← 模态：独立的根
```

**模态对话框作为独立的根**，不挂在 `Workspace` 下面。这样 Workspace 层的全局快捷键在模态中天然不匹配，复现了现在“对话框拦截所有按键”的行为，而不需要特殊判断。

### 3.4 Action

使用编译期枚举，不做动态注册表：Loom 没有扩展系统，用枚举可以让编译器检查每个 Action 都有处理器。

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    // workspace::
    ToggleDrawer, OpenExplorer, OpenSourceControl, ToggleTerminal, OpenSettings,
    OpenKeymap, OpenKeymapFile, CloneRepository, CloseOverlay,
    // pane::
    ActivateNextItem, ActivatePrevItem, ActivateItem(usize), ActivateLastItem, CloseActiveItem,
    // editor::
    Editor(EditorAction),          // Move/Select/Delete/Undo/Copy/Save/...
    // terminal::
    Terminal(TerminalAction),      // Copy, CopyOrInterrupt, Paste, SendKeystroke(Keystroke)
    // menu:: / dialog::
    MenuCancel, DialogConfirm, DialogCancel, DialogSecondary,
    // 特殊
    NoAction,                      // JSON 中的 null
}

pub struct ActionMeta { pub name: &'static str, pub doc: &'static str, pub takes_arg: bool }
pub const ACTIONS: &[ActionMeta] = &[ /* 所有 Action，供未知名报错、未来的命令面板和补全使用 */ ];

impl Action {
    pub fn name(&self) -> &'static str;
    pub fn from_json(value: &serde_json::Value) -> Result<Self, ActionError>; // "x" | ["x", arg] | null
}
```

命名遵循 Zed 的习惯（`PascalCase`，命名空间与上下文对应），方便用户直接参考 Zed 的配置。

### 3.5 Keymap 与优先级

```rust
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum BindingSource { User = 0, Base = 1, Default = 2 }   // 数值越小越强，和 Zed 的 meta 一致

pub struct Binding {
    pub keystrokes: SmallVec<[Keystroke; 2]>,   // 不想引入 smallvec 依赖时可直接用 Vec
    pub action: Action,
    pub predicate: Option<Rc<ContextPredicate>>,
    pub source: BindingSource,
}

pub struct Keymap { bindings: Vec<Binding>, by_action: HashMap<&'static str, Vec<usize>>, version: u64 }

impl Keymap {
    /// 移植 Zed 的 resolve_bindings_for_input：返回按优先级排列的 Action 列表，以及是否 pending
    pub fn bindings_for_input(&self, input: &[Keystroke], stack: &[KeyContext]) -> (Vec<&Binding>, bool);
    /// 显示用：在给定上下文中真正会触发该 Action 的按键
    pub fn binding_for_action(&self, action: &Action, stack: &[KeyContext]) -> Option<&Binding>;
}
```

优先级规则与 Zed 完全一致（见 1.4 节）。第一期不实现 `unbind` 段，只支持 `null`；`BindingSource::Base` 先预留。

### 3.6 分发：KeyDispatcher

> **实现调整**：最终没有做处理器注册表，而是改为集中分发。原因有二：lgui 的事件闭包要求 `Send + Sync`，注册表需要 `Arc<Mutex<..>>`，复杂度不划算；多键序列超时后，flush 发生在事件回调之外，拿不到 `UiEventContext`。
>
> 实际结构：
> - `src/input/dispatcher.rs`：纯函数状态机（`dispatch_key` / `flush` / `replay_prefix`）。
> - `src/key_actions.rs`：每帧在 `app.rs` 构造一份 `KeyEnv`，里面有 state、`ApplicationContext`（提供剪贴板和窗口）、编辑器焦点和区域、终端 controller。`perform(env, action) -> bool` 按命名空间集中执行，返回 `false` 时回落到下一个绑定（对应 Zed 的 propagate）。
> - pending 按键存放在 `AppState.pending_keystrokes` 中。超时由 worker 线程计时，然后通过 `ApplicationHandle::post` 回到 UI 线程执行 flush。
>
> 下面是原设计，保留作对照。

lgui 没有 `on_action`，因此用**每帧重建的处理器表**来模拟“沿焦点路径冒泡”：

```rust
pub enum Handled { Yes, Propagate }
type Handler = Rc<dyn Fn(&Action, &mut EventContext) -> Handled>;

pub struct KeyDispatcher {
    keymap: Rc<Keymap>,
    handlers: Vec<(ContextName, Handler)>,     // 在 render 期间注册，每帧开始时清空
    pending: Option<Pending>,                  // { keystrokes, deadline, fallback_bindings }
}
```

- 每个视图在 render 中注册自己负责的命名空间。例如编辑器注册 `editor::*`：闭包里捕获 `rect`、clipboard 和 state，复用现在的 `execute_command`。root 注册 `workspace::*` 和 `pane::*`（调用 `AppState::apply`），终端注册 `terminal::*`，对话框注册 `dialog::*`。
- 分发入口：root 的 KeyDown **捕获阶段**（合并进 `app.rs:458` 已有的捕获处理）：

```
on KeyDown capture:
  if IME 预编辑中（editor.preedit 非空）→ return（放行）
  ks = Keystroke::from_event(ev)?          // 纯修饰键 → return
  stack = context_stack(&app)
  match dispatcher.dispatch(ks, &stack):
    Pending      → prevent_default + stop_propagation，状态栏显示 pending 的按键
    Matched(list)→ 依次调用处理器，直到返回 Handled::Yes；
                   任一处理后 prevent_default + stop_propagation；
                   全部 Propagate → 当作未绑定处理
    Unbound      → return（交给 editor 的文本输入、终端的 key_bytes、Input 组件）
    Replay(list) → 先执行或回放旧的 pending 键，再处理当前键
```

- **未绑定的键放行**：编辑器的 `on_key_down` 只保留文本插入和 IME；终端的 `key_bytes` 保持不变；Input 组件的文本编辑键保持不变（第一期不迁移）。
- **终端兼容**：现在 Ctrl+B / Ctrl+W 等在终端中会发给 shell。默认 keymap 的 `Terminal` 段对这些键写 `null`。按 Zed 的语义，null 屏蔽了更浅层的 Workspace 绑定，键就落到终端自己的处理。具体名单在 M4 中对照 `key_bytes` 逐键确认。
- **Pending 超时**：默认 1000 ms。到期后需要唤醒事件循环来执行 `flush`。实现方式：后台线程 sleep 后调用 `request_frame`，在下一帧检查 deadline。M6 前需确认 lgui 是否有定时唤醒 API，没有就用这个兜底方案。
- **回放**：未匹配的可打印键交给编辑器的插入文本处理器（`editor::ReplayText`，内部 Action，不写进配置），其他键丢弃。第一期的默认 keymap 不会出现以可打印字符开头的序列，所以回放只在用户自定义时才会触发。

### 3.7 Keymap 文件

**默认配置** `assets/keymaps/default.json`（用 `include_str!` 编进程序，加载失败直接 panic 并由测试保证），完整保留现有行为：

```jsonc
[
  {
    "context": "Workspace",
    "bindings": {
      "secondary-b": "workspace::ToggleDrawer",
      "secondary-shift-g": "workspace::OpenSourceControl",
      "secondary-`": "workspace::ToggleTerminal",
      "secondary-,": "workspace::OpenSettings",
      "secondary-w": "pane::CloseActiveItem",
      "ctrl-tab": "pane::ActivateNextItem",
      "ctrl-shift-tab": "pane::ActivatePrevItem",
      "secondary-1": ["pane::ActivateItem", 0],
      // ... secondary-2 ~ secondary-8
      "secondary-9": "pane::ActivateLastItem",
      "escape": "workspace::CloseOverlay"
    }
  },
  {
    "context": "Editor",
    "bindings": {
      "secondary-s": "editor::Save",
      "secondary-z": "editor::Undo",
      "secondary-shift-z": "editor::Redo",
      "secondary-y": "editor::Redo",
      "secondary-a": "editor::SelectAll",
      "secondary-c": "editor::Copy",
      "secondary-x": "editor::Cut",
      "secondary-v": "editor::Paste",
      "left": "editor::MoveLeft",            "shift-left": "editor::SelectLeft",
      "right": "editor::MoveRight",          "shift-right": "editor::SelectRight",
      "up": "editor::MoveUp",                "shift-up": "editor::SelectUp",
      "down": "editor::MoveDown",            "shift-down": "editor::SelectDown",
      "secondary-left": "editor::MoveToPreviousWordStart",
      "secondary-right": "editor::MoveToNextWordEnd",
      "home": "editor::MoveToBeginningOfLine",
      "end": "editor::MoveToEndOfLine",
      "secondary-home": "editor::MoveToBeginning",
      "secondary-end": "editor::MoveToEnd",
      "pageup": "editor::MovePageUp",        "pagedown": "editor::MovePageDown",
      // shift- 变体对应 Select*
      "backspace": "editor::Backspace",
      "delete": "editor::Delete",
      "secondary-backspace": "editor::DeleteToPreviousWordStart",
      "secondary-delete": "editor::DeleteToNextWordEnd",
      "enter": "editor::Newline",
      "tab": "editor::Tab",
      "shift-tab": "editor::Backtab",
      "escape": "editor::Cancel"
    }
  },
  {
    "context": "Terminal",
    "bindings": {
      "ctrl-c": "terminal::CopyOrInterrupt",
      "ctrl-shift-c": "terminal::Copy",
      "ctrl-v": "terminal::Paste",
      "shift-insert": "terminal::Paste",
      "ctrl-b": null,                        // 让给 shell，保持现有行为
      "ctrl-w": null
      // 其余 null 名单在 M4 对照 key_bytes 确定
    }
  },
  { "context": "Menu", "bindings": { "escape": "menu::Cancel" } },
  {
    "context": "CloseConfirmation",
    "bindings": {
      "enter": "dialog::Confirm",
      "escape": "dialog::Cancel",
      "secondary-d": "dialog::Secondary"
    }
  },
  {
    "context": "CloneRepository",
    "bindings": { "enter": "dialog::Confirm", "escape": "dialog::Cancel" }
  }
]
```

> 已知行为变化：现在编辑器获得焦点时，Esc 只清除选区，不会关闭编辑器菜单。新方案中菜单打开时 `Menu` 是最深层上下文，所以 Esc 会先关闭菜单。这是修正，不是回归。

**用户配置** `%APPDATA%\Loom\keymap.json`

- 用 JSONC 预处理函数去掉 `//`、`/* */` 注释和尾逗号（约 60 行，带测试），再交给 `serde_json`，不引入新依赖。
- 宽松反序列化：每条绑定单独报错，汇总后用 toast 展示前几条错误。JSON 语法错误时保留上一份有效的 keymap。
- 未知字段、未知 Action 名、按键解析失败：跳过该条并记录错误。
- 热重载：用 `notify` 监听配置目录中的 `keymap.json`（独立于工作区的 watcher），去抖后重建 `Keymap` 并递增 `version`。
- `workspace::OpenKeymapFile`：文件不存在时写入带注释的模板，然后在编辑器标签页中打开。

### 3.8 显示文字

- `keymap.binding_for_action(&action, &stack)` 返回 `Option<&Binding>`，再由 `Keystroke::display()` 生成文字。
- 将 `edit_menu.rs`、`context_menu.rs` 中写死的快捷键文字改为反查（`stack` 取 `[Workspace, Editor]` 这类该菜单所在的上下文）。
- 显示格式：Windows/Linux 为 `Ctrl+Shift+G`，序列之间用空格；macOS 为 `⌘⇧G`。

### 3.9 调试与可观测性

- `dev::ShowKeyContext`（第二期）：浮层显示当前 context stack 和最后一次按键匹配的结果，相当于 Zed 的 KeyContextView。
- 第一期至少在 debug 构建中打印日志：`keystroke → stack → 命中的绑定或未绑定`。

### 3.10 测试策略

| 层 | 方式 |
|---|---|
| keystroke / context / keymap | 纯单元测试；移植 Zed `keymap.rs`、`context.rs` 中的用例（深度优先级、null 的作用范围、pending、Not / > 语义） |
| keymap_file | 解析 `default.json` 必须零错误；部分失败；JSONC；未知 Action |
| 行为一致性 | 把现有 `keymap.rs` 和 `commands.rs` 的测试改写为“`Keystroke` + 上下文 → Action”，结果必须与旧实现一致 |
| 集成 | 复用 `editor_view.rs` 测试中的 lgui 无窗口事件分发工具，覆盖：编辑器 Ctrl+S、终端 Ctrl+C / Ctrl+B 透传、对话框打开时 Ctrl+B 无效、IME 预编辑中不触发 |
| 手动回归 | 见下方各里程碑的验收项 |

---

## 4. 实施规划

### M0 调研与设计

- [x] 梳理 Loom 现有的按键处理链路（keymap / commands / terminal / dialogs / input）
- [x] 阅读 Zed 的 keymap、context、keystroke、key_dispatch、keymap_file 源码和文档
- [x] 输出本设计文档

### M1 基础类型：Keystroke 与 Context

- [x] `input/keystroke.rs`：`Modifiers`、`Keystroke::parse` / `parse_sequence`（含 `secondary-`、命名键表、别名、`ctrl--`）
- [x] `Keystroke::from_event`：shift 和标点规则、大小写规范化、AltGr 处理（Windows 上 Ctrl+Alt 产生的非字母数字字符记为 `key_char`）、忽略纯修饰键和按键释放
- [x] `Keystroke::should_match`、`label()` / `sequence_label()`（Windows/Linux 与 macOS 两种格式）、`Display`（可还原为 keymap 语法）
- [x] `input/context.rs`：`KeyContext` 与 `ContextPredicate` 的解析器（`&& || ! == != > ()`）
- [x] `depth_of` / `eval_inner` / `is_superset`，语义与 Zed v0.197 之后一致
- [x] 单元测试：移植 Zed 的 context 测试，加上 Loom 自己的 keystroke 测试

**验收**：`cargo test input::` 全部通过。✅

### M2 Keymap 与文件解析

- [x] `input/keymap.rs`：`Binding`、`BindingSource { User, Default }`、`Keymap::bindings_for_input`（深度和顺序排序、NoAction 来源规则、pending 判定）
- [x] `Keymap::bindings_for_action` / `binding_for_action` / `active_bindings`；`current()` / `install()` 管理当前生效的 keymap
- [x] `input/keymap_file.rs`：JSONC 预处理（保持行号）、宽松逐条解析；`parse` 返回 `Err`（整个文件不可用）或 `LoadOutcome { bindings, errors }`
- [x] 单元测试：移植 Zed 的深度优先级、null、来源强弱、多键 null、前缀与 pending 等用例
- [x] `serde_json` 开启 `preserve_order`，同一段内的绑定按文件中的书写顺序生效（与 Zed 的 IndexMap 一致）

**验收**：给定 JSON、上下文和按键序列，能得到与 Zed 规则一致的结果。✅

### M3 Action 统一与默认 keymap

- [x] `input/action.rs`：`Action` 枚举（宏生成名字表）、`ACTIONS` 元数据（名字和描述）、`from_json` / `name` / `namespace`、`editor_command`
- [x] 合并原有的 `keymap::Action` 和 `editor::commands::Command` 的语义。`Command` 降级为编辑器内部执行原语；`command_for` 仅保留给 Input 组件使用（Input 本期不迁移）
- [x] `assets/keymaps/default.json`：完整覆盖现有快捷键（见 3.7）
- [x] 测试：`default.json` 零错误；`context_stack.rs` 中的一致性测试逐键对照旧的 `action_for` / `command_for` / 终端剪贴板 / 对话框行为

**验收**：默认 keymap 在各上下文下给出的 Action 与旧实现逐键一致。✅（例外见第 5 节“有意的行为变化”）

### M4 统一分发与迁移（风险最高）

- [x] `input/context_stack.rs`：由 `AppState` 推导上下文栈（Workspace os/surface → Editor extension / Terminal / Input → Menu；模态 Dialog 作为独立的根）
- [x] `input/dispatcher.rs`：纯函数状态机；`key_actions::perform` 集中执行，返回 `false` 时回落到下一个绑定（与原设计的差异见 3.6）
- [x] 在 `app.rs` 的 KeyDown 捕获阶段接入（`key_actions::attach`）；删除 root 冒泡阶段的 `action_for` 和终端剪贴板的捕获处理
- [x] 编辑器：删除 `command_for` 和 `is_save_shortcut` 的按键解析；`on_key_down` 只保留“未绑定的 Ctrl 组合键不输入文字”；抽出 `insert_text` 和 `page_lines`
- [x] 终端：剪贴板处理改为 `terminal::*` Action（`run_clipboard_shortcut`）。对照 `key_bytes` 确定的 `null` 名单：`ctrl-w`、`escape`、`ctrl-tab`、`ctrl-shift-tab`、`secondary-k secondary-s`（让 Ctrl+K 留给 shell）
- [x] 关闭确认对话框、克隆对话框：改为 `dialog::*` Action（`close_confirmation::perform` / `clone_repository::perform`），模态作为独立的上下文根
- [x] 菜单打开时加入 `Menu` 上下文，Esc 触发 `menu::Cancel`
- [x] `Input` 上下文（Explorer 新建或重命名、提交信息输入框）：`escape: null`，让输入框自己处理取消
- [x] 集成测试：`editor_view` 的无窗口事件测试改为经由 `key_actions::attach` 分发（覆盖 Tab、Ctrl+Z、IME 预编辑和提交）；终端、对话框、Input 的路由在 `context_stack` 测试中以“事件 → 上下文 → Action”的方式覆盖

**验收（手动回归清单，需在真实窗口中验证）**

- [ ] 编辑器：方向键、选择、词级移动和删除、Home/End、PageUp/Down、Tab/Shift+Tab、撤销/重做、剪贴板、Ctrl+S
- [ ] IME 中文输入：预编辑过程中按方向键、Enter、Backspace 不触发 Action
- [ ] 终端：Ctrl+C（有选区时复制，否则中断）、Ctrl+Shift+C、Ctrl+V、Shift+Insert、Ctrl+W / Ctrl+K / Esc 发给 shell、Ctrl+B 仍切换 Explorer、方向键序列
- [ ] 全局：Ctrl+B、Ctrl+Shift+G、Ctrl+`、Ctrl+W、Ctrl+,、Ctrl+1~9、Ctrl+Tab / Ctrl+Shift+Tab、Esc
- [ ] 对话框：关闭确认的 Enter / Esc / Ctrl+D；克隆对话框的 Enter / Esc / 输入 / Backspace；对话框打开时全局快捷键无效
- [ ] Explorer 新建或重命名输入框、提交信息输入框：Esc 取消、Enter 提交、Ctrl+A/C/V 作用于输入框
- [ ] Settings 页、Keymap 页、Diff 页、Welcome 页下全局快捷键正常；Keymap 页筛选框中 Esc 清空、Ctrl+A/C/V 作用于输入框

### M5 用户 keymap

- [x] 读取 `config_path("keymap.json")`，以 `BindingSource::User` 追加在默认配置之后（`keymap_file::build`）
- [x] 部分失败时用 toast 提示（第一条错误加剩余条数）；JSON 语法错误时保留旧 keymap
- [x] 用 `notify` 监听配置目录并热重载（200 ms 去抖，`key_actions::watch_user_keymap`）；重载后递增 `AppState.keymap_version`，触发界面上的快捷键文字刷新
- [x] `workspace::OpenKeymapFile` Action：生成带注释的模板并在标签页中打开（Keymap 页的“Edit keymap.json”按钮）
- [x] README 中补充 keymap 文档（语法、上下文、优先级、示例）

**验收**：修改 `keymap.json` 后无需重启即可生效；写入非法内容时有提示，且原有快捷键仍然可用。（逻辑由单元测试覆盖，热重载需手动验证）

### M6 多键序列

- [x] Pending 状态机：前缀匹配时等待，超时 1000 ms 后 flush（`dispatcher::PENDING_TIMEOUT`）
- [x] Replay：旧 pending 中最长可执行前缀先执行；其余键在 Editor 中回放为文本，在 Terminal 中写入 pty，其他上下文丢弃
- [x] 定时唤醒：lgui 没有定时 API，改用 worker 线程计时，再用 `ApplicationHandle::post` 回到 UI 线程；窗口失焦时清空 pending
- [x] 状态栏显示 pending 的按键（如 `Ctrl+K …`）
- [x] 测试：前缀和完整绑定并存、超时 flush、中途按错键时回放已绑定的前缀或文本

**验收**：`secondary-k secondary-s` 能打开 Keymap 页；按下 `secondary-k` 后按其他键，那个键按原有方式生效。（逻辑由单元测试覆盖，需手动确认）

### M7 显示与可发现性

- [x] `edit_menu.rs` 的快捷键文字改为从 keymap 反查（`key_actions::editor_shortcut_label`）。`context_menu.rs` 目前没有任何项显示快捷键，无需改动
- [x] 独立的 **Keymap 页**（与 Settings 一样以单独标签页打开，`workspace::OpenKeymap`，默认绑定 `secondary-k secondary-s`，标签图标为 ⌘（`command`；键盘图标在 12px 下细节过多））：只读列出全部 Action 及其绑定（Action、按键、上下文、来源，User 来源高亮），提供“Edit keymap.json”按钮和显示配置文件路径
- [x] Keymap 页的筛选框：按空格分词，对 Action、描述、按键、上下文、来源做不区分大小写的匹配；Esc 清空筛选
- [x] Settings 页只保留 KEYBOARD 分区的一行入口（“Open Keymap”）
- [x] 工作区模型把 `settings: bool` 泛化为 `page: Option<AppPage>`（Settings / Keymap），每种页面最多一个标签
- [x] debug 构建中设置 `LOOM_LOG_KEYS=1` 后打印按键匹配日志

**验收**：用户改绑 `editor::Undo` 后，编辑菜单上显示的是新的按键。✅（`binding_for_action` 单元测试覆盖）

### 后续（第二期候选）

- [ ] 命令面板（基于 `ACTIONS` 元数据和 `binding_for_action`）
- [ ] Keymap 编辑器：录制按键、冲突检测、写回 `keymap.json`
- [ ] `unbind` 段与 `Unbind` 语义
- [ ] Base keymap 预置方案（VSCode / JetBrains），对应 `BindingSource::Base`
- [ ] `workspace::SendKeystrokes`、`terminal::SendKeystroke`
- [ ] 文件树焦点与 `Explorer` 上下文（重命名、删除、新建等快捷键）
- [ ] Input 组件迁移到 keymap（移除 `commands::command_for`）
- [ ] 更多上下文属性：`has_selection`、`mode`（`extension`、`os`、`surface` 已提供）
- [ ] `dev::ShowKeyContext` 调试浮层
- [ ] 非 QWERTY 布局（key equivalents）

---

## 5. 风险与待决问题

| 风险或问题 | 应对 |
|---|---|
| 捕获阶段统一分发后，按键可能被意外拦截（终端、IME、输入框） | 只有命中绑定才 `stop_propagation`；M4 的回归清单；终端用 `null` 名单显式让出按键 |
| `secondary-` 让 macOS 上的 Ctrl+B 等不再触发（现在 Ctrl 和 Meta 都接受） | 有意收敛到平台惯例；如有需要可在默认配置中用 `os == macos` 的段补充 |
| lgui 的 `KeyboardEvent` 是否区分 AltGr、是否带 repeat 标志、是否有物理按键码 | 已确认（lgui `72a4cb0`，基于 keyboard-types 0.8）：有 `repeat`、`is_composing`、`code`（物理键）；**没有** AltGr 标志，Windows 上的 AltGr 表现为 Ctrl+Alt，按 3.2 的规则兜底 |
| pending 超时需要定时唤醒 | 已解决：lgui 没有定时 API，用 worker 线程加 `ApplicationHandle::post` |
| 每帧重建 `KeyEnv` 的开销 | 只是几个句柄的 clone；Keymap 用 `Arc` 共享，只在重载时重建；读取上下文时用 `try_update` 借用 `AppState`，不整体 clone |
| Input 组件的文本编辑键暂不迁移 | 第一期保持组件自行处理（`commands::command_for`），只引入 `Input` 上下文来让出 Esc |

**有意的行为变化**（与旧实现不同，均已写进默认配置或测试）

1. 平台修饰键按平台各自生效：Windows/Linux 上 Win/Super 键不再等同 Ctrl；macOS 上 Ctrl 不再等同 Cmd。
2. 菜单打开时 Esc 先关闭菜单（`Menu` 上下文），包括终端获得焦点的情况。
3. 克隆对话框打开时，全局快捷键（如 Ctrl+B）不再生效（模态根）。原来只有关闭确认对话框是模态的。
4. 编辑器中按 Ctrl+K 会进入 pending，等待 `Ctrl+S` 组成 `OpenKeymap`；超时后丢弃（原来 Ctrl+K 本就没有功能）。终端中 Ctrl+K 仍发给 shell。
5. 编辑器中 Ctrl+Shift+A/C/X/V 不再等同 Ctrl+A/C/X/V（原来大小写不敏感的副作用）；Ctrl+Shift+Z 仍然是 Redo。

---

## 6. 参考

- Zed 文档：<https://zed.dev/docs/key-bindings>
- `crates/gpui/src/keymap.rs`：优先级、NoAction / Unbind、pending、`possible_next_bindings_for_input`
- `crates/gpui/src/keymap/context.rs`：`KeyContext`、谓词解析、`depth_of`
- `crates/gpui/src/keymap/binding.rs`：`KeyBinding`、`match_keystrokes`
- `crates/gpui/src/platform/keystroke.rs`：`Keystroke`、`should_match`、显示
- `crates/gpui/src/key_dispatch.rs`：`dispatch_key`、`replay_prefix`、`flush_dispatch`
- `crates/settings/src/keymap_file.rs`：`KeymapSection`、部分失败加载、`KeybindSource`
- 默认键位：`assets/keymaps/default-{macos,windows,linux}.json`
