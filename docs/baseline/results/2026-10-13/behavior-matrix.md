# Loom 当前行为基线矩阵

基于 5ade296 和 2026 年 10 月 13 日 Windows 测试结果。以下是当前应保持的行为，不是阶段一后的新设计。完整入口名和状态见 test-inventory.json。

## 编辑与视图

| 行为 | 当前约定 | 代表测试 | 本轮验证 |
| --- | --- | --- | --- |
| Unicode 编辑 | 中文、Emoji 和组合字素安全移动/删除，非法簇内位置吸附 | baseline_tests::grapheme_clusters_move_and_delete_as_one_character | 自动通过 |
| 换行 | LF/CRLF 保留；CRLF 导航与删除视为一个换行 | baseline_tests::crlf_is_one_line_break_for_navigation_and_deletion | 自动通过 |
| 撤销分组 | 连续输入合并；移动、粘贴、IME 等分组边界保持 | committed_text_merges_into_one_undo_step_until_the_caret_moves；ime_commit_is_its_own_undo_step | 自动通过 |
| 跨窗格 | 同一文档共享文本和历史，各视图选区/滚动独立 | edits_through_one_pane_keep_the_other_panes_selection_on_the_same_text；undo_restores_the_undoing_pane_and_shifts_the_other_panes | 自动通过 |
| dirty | 编辑变脏，撤销回保存文本后变 clean | saving_and_undoing_back_to_saved_text_tracks_dirty_state | 自动通过 |
| 剪贴板失败 | Cut/Paste 失败不破坏文本、选区或预览状态 | clipboard_failure_and_empty_clipboard_never_destroy_selection；a_failed_cut_does_not_promote_or_modify_a_preview | 自动通过 |
| IME 目标 | preedit 不写文本；晚到提交进入原视图；已关闭目标丢弃 | a_late_composition_commit_lands_in_the_view_it_started_in；a_commit_for_a_closed_view_is_dropped | 事件测试通过，原生候选窗口待验 |
| Vim | 通过真实事件路径处理模式、重复、映射和应用快捷键 | editor::vim_integration_tests；vim::tests | 自动通过 |
| 长行 | 水平视口切片与命中保持边界，文件内容不截断 | multi_megabyte_single_line_windows_and_clicks_are_bounded；long_unicode_windows_remain_on_grapheme_boundaries_near_the_end | 自动通过及 release 采样 |

## 文件 工作区与会话

| 行为 | 当前约定 | 代表测试 | 本轮验证 |
| --- | --- | --- | --- |
| 文件大小与编码 | UTF-8 编辑文件上限为 64 MiB，边界包含；拒绝目录与非法 UTF-8 | text_file_size_boundary_is_inclusive；text_file_rejects_invalid_utf8_and_directories | 自动通过 |
| 预览标签 | 单击预览，双击或编辑提升；导航不提升；干净旧预览可替换 | editing_commands_promote_a_preview_while_navigation_does_not；a_new_preview_replaces_the_clean_preview_at_the_same_index | 自动通过 |
| 异步打开 | 保留当前画面，300 ms 后才显示加载图标；新请求或已有标签切换取消旧请求 | slow_open_keeps_current_screen_interactive_and_delays_only_the_icon；last_selection_wins_and_a_cancelled_result_cannot_replace_the_screen | 自动通过 |
| 过期结果 | 工作区改变后旧结果不能重开文件，晚到错误不覆盖当前标签 | changing_workspaces_discards_a_prepared_result_without_reopening_it；returning_to_an_existing_tab_cancels_a_load_and_its_late_error | 自动通过 |
| 加载期间关闭/恢复 | 先有标签和顺序，后台填充内容；关闭项不恢复，Loading 不允许编辑保存 | session_restore::tests；file_loading::tests | 自动通过 |
| 外部变更 | clean 文档可重载，dirty 文本保留并标记冲突；晚到磁盘快照不覆盖新编辑/保存 | dirty_document_is_never_replaced_during_disk_reconciliation；delayed_disk_notification_preserves_intervening_edits_and_saves | 自动通过 |
| 目录监视 | 重复事件归并；过量事件全量重扫；树缓存收敛，行内编辑时延迟刷新 | file_watcher::tests；file_tree::tests | 自动通过 |
| 重命名 | 更新打开文档的路径身份，不重建编辑文档 | directory_rename_retargets_open_files_without_reopening_them；renamed_file_retargets_its_open_editor_tab | 自动通过 |
| 关闭选择 | Save/Don't Save/Cancel 保持原语义；最后视图关闭才处理脏文档 | closing_one_of_two_tabs_of_a_dirty_document_needs_no_confirmation；cancelling_batch_close_preserves_dirty_tabs | 自动通过 |
| 批量保存失败 | 保存失败时所有标签继续存在 | failed_batch_save_keeps_every_tab_open | 自动通过 |
| 退出会话 | Save/Don't Save 的退出过程保留标签直到最终快照 | saving_for_application_exit_preserves_tabs_for_the_final_snapshot；final_session_snapshot_includes_pending_and_just_loaded_files_without_a_render | 自动通过 |
| 旧会话兼容 | 缺失字段采用默认值；分屏顺序、活动项、终端并列数组保留 | 新增 phase0_legacy_session_defaults_fixture；phase0_split_session_fixture_preserves_order_and_terminal_metadata | 夹具通过；磁盘恢复另由现有流程测试覆盖 |
| 文件拖放 | 根据落点插入、移入或分屏；失败不改变工作区 | dropping_a_file_on_a_strip_inserts_it_at_the_marker；dropping_a_file_near_a_pane_edge_opens_it_in_a_new_split | 自动通过 |

## 搜索 Git 与终端

| 行为 | 当前约定 | 代表测试 | 本轮验证 |
| --- | --- | --- | --- |
| 搜索范围 | UTF-8，每文件 4 MiB，上限 10,000 匹配/5,000 结果文件；遵守 ignore、glob、hidden 和 binary 规则 | filters_whole_word_zero_width_and_result_limit；unopened_nested_files_ignore_filters_and_buffer_override | 自动通过 |
| 未保存文本 | 已打开 buffer 优先，结果导航检验匹配位置 | older_search_notifications_cannot_replace_new_query_and_navigation_uses_buffer | 自动通过 |
| 搜索生命周期 | 输入 debounce 200 ms，取消和旧 generation 不覆盖新查询；面板切换保留结果 | query_key_tracks_search_inputs_and_workspace_edits_but_not_view_state；search_effect_preserves_results_across_unrelated_ui_and_drawer_changes | 自动通过，主动取消采样通过 |
| Git 基本操作 | status、stage/unstage、commit、history、Diff 与 blame 保持 | real_repository_status_stage_commit_history_diff_and_blame | 真实临时仓库通过 |
| Git 换行环境 | 测试夹具预期 LF，系统 autocrlf 会改变 checkout 的工作树换行 | real_repository_resolves_conflicts_and_continues_a_rebase；real_repository_switches_with_stashed_changes | 默认失败，隔离配置通过，已登记 |
| Git 后同步 | 操作后 clean 文件可更新，未保存 buffer 保留；Diff 刷新保持标签/视图 | polling_reloads_open_diffs_after_edits_stage_commit_and_deletion；background_diff_refresh_preserves_tabs_focus_and_views | 自动通过 |
| Git 工作树 | 创建、选择和删除保护保持，不切换原仓库工作树 | real_repository_manages_worktrees_without_switching_the_source | 自动通过 |
| Git Diff 拖放 | 保留 staged/working-tree 目标，按目标窗格插入或分屏 | git_actions::tests 的 diff_drop 用例 | 自动通过 |
| Git 面板 | 上部变更与下部详情分区，展开时仍为变更保留空间 | ui::git_panel 的布局及事件测试 | 自动通过，原生外观待验 |
| 终端基本交互 | ANSI/Unicode/wide cells、scrollback、选择、bracketed paste 保持 | ansi_parser_produces_colored_runs；wide_terminal_cells_keep_their_two_column_positions；paste_normalizes_newlines_and_honors_bracketed_mode | 自动通过及 parser 采样 |
| shell 与 cwd | profile 参数保留；不可用 default/profile fallback；cd 目录恢复与继承 | custom_profiles_preserve_arguments_split_configuration_and_missing_path_fallback；real_shell_directories_follow_cd_and_survive_session_restoration | 真实 shell 测试通过 |
| 终端分组/布局 | 分组成员独立，移动组不重建 controller 或丢失 scrollback/cwd/selection | moving_a_group_leader_preserves_controller_cwd_scrollback_and_selection | 自动通过 |
| 原生 ConPTY | shell round-trip、mouse record、launcher、Git Bash 替换和程序名检测 | supported_shells_round_trip_through_real_conpty_sessions 与三项 process smoke；conpty_delivers_wheels_as_native_mouse_records_and_restores_history | 全部已执行通过 |

## 设置与输入

| 行为 | 当前约定 | 代表测试 | 本轮验证 |
| --- | --- | --- | --- |
| JSONC | 注释/尾逗号解析，保留有效成员；文件修改保留原有注释和布局 | settings_persistence::file_tests；jsonc::tests；新增设置夹具 | 自动通过；新增夹具仅测解析，不写用户文件 |
| 配置作用域 | 项目只覆盖现有白名单 default_shell，不应用用户专属设置 | project_settings::tests::only_project_keys_apply；新增设置 fixture scope 断言 | 自动通过 |
| 快捷键兼容 | 动作名、参数、null 禁用、上下文优先级和用户后置规则保持 | input::keymap 和 keymap_file 测试；新增 Keymap 夹具 | 自动通过 |
| 序列分发 | 多键等待、超时、放弃序列的绑定/文本重放保持 | input::dispatcher::tests | 自动通过 |
| 焦点与模态 | Terminal、Input、Editor、Vim 和 Dialog 上下文正确隔离 | input::context_stack::tests；editor::vim_integration_tests | 事件测试通过，系统原生切换待验 |

## 必须另外执行的人工检查

本轮没有操作真实桌面窗口，因此以下清单为待执行项，不计为通过。

1. 在中文输入法下检查 preedit、候选框坐标、切换窗格后晚到提交和失焦；确认系统候选窗口始终锚定正确视图。
2. 检查 SVG 文件标签图标、加载占位、300 ms 指示器和小尺寸窗口的视觉布局。
3. 从 Editor、Terminal 和输入框分别打开/关闭确认框与菜单，使用原生窗口焦点切换确认恢复目标。
4. 打开长行/Unicode/大小限制附近的文件，实际滚动、点击和输入；测量软件渲染及窗口呈现，不能把 UI session 时间直接代替。
5. 在 Linux 和 macOS 复跑对应测试并记录 native PTY、系统字体、窗口事件差异。
6. 如后续迁移改变保存/回收站/退出协调，使用 synthetic 工作区验证真实 OS 对话框和回收站取消路径，避免用真实用户文件做回归材料。

人工完成后补充环境、步骤、预期、实际结果和时间。当前自动测试结果与人工待验状态分开保留。
