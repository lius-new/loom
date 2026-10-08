# 编辑核心基线（阶段一）

本文记录编辑核心重构开始时的编辑入口、基线测试和性能基线。代码位置以提交 `4c89c52` 为准。

## 1. 入口清单

### 1.1 代码编辑器

| 入口 | 触发来源 | 路径 | 修改内容 |
| --- | --- | --- | --- |
| 键盘编辑命令 | 按键映射中的 `editor::*` 动作 | `key_actions::perform` → `Action::editor_command` → `editor_view::execute_command` → `apply_command` → `commands::apply` → `EditorMut::{navigate, erase, enter, indent, select_all, clear_selection, undo, redo}` | 文本、选区、历史 |
| 剪贴板命令 | `editor::Copy/Cut/Paste` 动作、上下文菜单 | `apply_command` 中直接读写剪贴板，再调用 `EditorMut::{backspace, paste, break_undo_group}` | 文本、历史 |
| 上下文菜单 | 右键菜单项 | `edit_menu::render` → `execute_command`（与键盘命令同一路径） | 同上 |
| 已提交文本 | 平台文本输入（`on_input`），包括普通按键文本与输入法提交 | `editor_view::render` 的 `on_input` → `insert_text` → `EditorMut::insert` | 文本、历史（输入法提交前后断开分组） |
| 序列回放 | 被放弃的按键序列中未绑定的按键 | `key_actions::run_replay` → `insert_text` | 文本 |
| 输入法组合 | `on_composition_update/end` | 写入 `app.editor.{preedit, preedit_cursor, ime_pending}`（应用级，所有窗格共享） | 仅临时状态 |
| 指针选择 | 按下、拖动、按住滚动 | `on_pointer_down` → `EditorMut::{set_cursor, select_to, select_range}`；`on_pointer_drag`、`drag_scroll_tick` → `update_drag_selection` | 选区，断开历史分组 |
| 失去焦点 | `on_blur` | 清空输入法状态，`EditorMut::break_undo_group` | 历史分组 |
| 保存 | `workspace_actions::save_active_document` 等 | `Workspace::mark_saved` → `TextBuffer::break_undo_group` | 历史分组、脏状态 |
| 外部文件变更 | 文件监视、接受磁盘版本 | `Workspace::{reconcile_document, accept_disk_version}` → `replace_buffer_from_disk` → `TextBuffer::reload` + `clamp_views` | 整体替换文本，重置历史，钳制所有视图选区 |
| 打开、分屏、移动标签 | 工作区操作 | `Workspace::{open_path, split, move_item, copy_item}` 创建或复制 `ViewState` | 视图选区与滚动 |

所有文本修改最终经过 `EditorMut::replace`（`insert`、`paste`、`erase`、`enter` 共用）或 `EditorMut::indent`（直接逐行改写文本），以及 `TextBuffer::reload`。`EditorMut::propagate` 负责把修改同步到同一文档的其他视图。

### 1.2 单行输入框

`ui/components/input.rs` 中的 `InputState`（资源管理器新建、提交信息、按键映射搜索等）自带 `TextBuffer` 和 `Selection`，通过无关联视图的 `EditorMut` 编辑：

- 按键：`on_key_down` → `commands::command_for`（硬编码按键表，不经过按键映射）→ `apply_command` → `commands::apply`。
- 文本：`on_input` → `EditorMut::insert`；`on_change` → `InputState::set_text`。
- 输入法组合状态保存在各自的 `InputState` 中。

## 2. 基线测试

除已有测试外，`src/editor/baseline_tests.rs` 通过界面使用的同一入口（编辑命令、已提交文本、工作区窗格编辑器）补充以下行为：

| 测试 | 覆盖的行为 |
| --- | --- |
| `crlf_is_one_line_break_for_navigation_and_deletion` | CRLF 下的行尾、跨换行移动、删除、上下移动和回车 |
| `committed_text_merges_into_one_undo_step_until_the_caret_moves` | 连续输入合并为一个撤销步骤，移动光标后断开 |
| `ime_commit_is_its_own_undo_step` | 输入法提交前后断开历史分组 |
| `cut_and_paste_are_separate_undo_steps_from_typing` | 剪切、粘贴各为一个撤销步骤，撤销恢复选区 |
| `edits_through_one_pane_keep_the_other_panes_selection_on_the_same_text` | 共享文档中其他窗格的选区随修改变换，被删除时收缩 |
| `undo_restores_the_undoing_pane_and_shifts_the_other_panes` | 从另一窗格撤销：发起撤销的窗格恢复记录的选区，其他窗格平移 |
| `pasting_follows_the_documents_line_endings` | 粘贴时换行符跟随文档约定 |
| `outdent_is_one_undo_step_and_each_word_deletion_is_another` | 多行缩进是一个撤销步骤，按词删除各自独立 |
| `saving_and_undoing_back_to_saved_text_tracks_dirty_state` | 撤销回保存时的文本后不再是脏状态 |
| `grapheme_clusters_move_and_delete_as_one_character` | 字素簇作为一个字符移动和删除，簇内位置吸附到起点 |

已有测试中与基线相关的部分：`model/buffer.rs`（Unicode、CRLF、历史、位置变换）、`editor/editor_view.rs`（拖选、快捷键与输入法提交的事件路由、剪贴板失败、预览提升）、`model/workspace.rs`（共享文档、独立选区与滚动、外部重载钳制选区、脏状态）、`ui/components/input.rs`（单行输入的提交文本）。

## 3. 性能基线

测量方法：`cargo test --release perf_baseline -- --ignored --nocapture`。文档由约 60 字节的代码行重复构成，在文档中部操作。数据于 2026-10-08 在开发机（Windows 11）上测得。

| 文档 | 逐字输入（每字符） | 粘贴 1 MB | 撤销粘贴 | 全选缩进 | 历史占用 |
| --- | --- | --- | --- | --- | --- |
| 1 MB | 17.8 ms | 19.4 ms | 1.3 ms | 152 ms | 2.0 MB |
| 10 MB | 178.6 ms | 70.4 ms | 9.7 ms | 24.2 s | 20.0 MB |

历史占用为撤销与重做快照的文本总量。

阶段二完成后（2026-10-08，同一方法）：

| 文档 | 逐字输入（每字符） | 粘贴 1 MB | 撤销粘贴 | 全选缩进 | 历史占用 |
| --- | --- | --- | --- | --- | --- |
| 1 MB | 19.5 µs | 17.7 ms | 0.4 ms | 16.9 ms | 0.0 MB |
| 10 MB | 198 µs | 17.7 ms | 0.8 ms | 168 ms | 0.3 MB |

历史改为记录精确变更后，占用为各步骤删除与插入文本的总量。

## 4. 发现的现有缺陷

重构开始时发现以下问题，均已在编辑核心重构中修复：

1. **逐字输入随文档大小线性变慢。** `TextBuffer::boundary`、`next` 从文档开头遍历字素；每次输入后 `reveal_cursor` 重新计算所有行和最长行宽。
2. **多行缩进是二次复杂度。** `EditorMut::indent` 对每一行分别调用 `replace_range` 并同步视图。
3. **输入法组合状态是应用级的。** 组合文本和提交目标不属于具体视图，提交时写入当时的活动文档。
4. **撤销时的反向变更由前后文本推算。** `TextEdit::between` 求最小差异，当被删除文本与相邻文本相同时，其他视图的位置可能落在相邻边界上。
