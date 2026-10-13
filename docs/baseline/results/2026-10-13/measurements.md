# 阶段零自动采样结果

样本零为预热，已排除。以下为同机 release 测试中位数和范围；UI session 数据不等于原生窗口帧率。

| 场景 | 指标 | 样本 | 单位 | 中位数 | 最小值 | 最大值 |
| --- | --- | --- | --- | --- | --- | --- |
| 8 MiB single line | ui_document_prepare | 5 | ms | 209.476200 | 202.916400 | 219.782100 |
| 8 MiB single line | ui_100_viewports_and_clicks | 5 | ms | 15.799500 | 15.288800 | 17.466700 |
| 8 MiB single line | ui_20_edits_and_renders | 5 | ms | 5.208000 | 5.051300 | 5.836800 |
| Unicode single line | ui_document_prepare | 5 | ms | 82.591400 | 81.945300 | 93.850200 |
| Unicode single line | ui_100_viewports_and_clicks | 5 | ms | 13.555800 | 13.245600 | 17.167800 |
| Unicode single line | ui_20_edits_and_renders | 5 | ms | 4.387200 | 4.253100 | 4.503500 |
| many short lines | ui_document_prepare | 5 | ms | 412.510300 | 402.303700 | 462.255500 |
| many short lines | ui_100_viewports_and_clicks | 5 | ms | 35.430500 | 32.750900 | 43.390500 |
| many short lines | ui_20_edits_and_renders | 5 | ms | 13.782400 | 13.312400 | 15.764400 |
| ascii-1m | typing_per_char | 5 | ms | 0.024783 | 0.024196 | 0.024942 |
| ascii-1m | paste_1m | 5 | ms | 29.033300 | 28.803400 | 29.295900 |
| ascii-1m | undo_paste | 5 | ms | 0.301300 | 0.271300 | 0.328400 |
| ascii-1m | select_all_indent | 5 | ms | 28.996000 | 27.972300 | 32.075500 |
| ascii-1m | history_bytes_rounded | 5 | bytes | 0.000000 | 0.000000 | 0.000000 |
| ascii-10m | typing_per_char | 5 | ms | 0.160189 | 0.152625 | 0.169506 |
| ascii-10m | paste_1m | 5 | ms | 29.334100 | 29.023600 | 33.314800 |
| ascii-10m | undo_paste | 5 | ms | 0.716600 | 0.701000 | 0.751600 |
| ascii-10m | select_all_indent | 5 | ms | 289.359900 | 285.002000 | 292.975000 |
| ascii-10m | history_bytes_rounded | 5 | bytes | 314572.800000 | 314572.800000 | 314572.800000 |
| 100-tabs | pointer_update | 5 | ms | 0.060629 | 0.060488 | 0.073842 |
| 500-tabs | pointer_update | 5 | ms | 0.383356 | 0.381143 | 0.398030 |
| ascii-1m | file_read_cached | 5 | ms | 0.725100 | 0.627300 | 1.052300 |
| ascii-1m | document_prepare | 5 | ms | 24.531700 | 23.191800 | 25.391600 |
| ascii-10m | file_read_cached | 5 | ms | 5.116400 | 4.911700 | 5.692400 |
| ascii-10m | document_prepare | 5 | ms | 244.790000 | 238.105900 | 255.496700 |
| ascii-64m | file_read_cached | 5 | ms | 37.752800 | 36.992500 | 38.709300 |
| ascii-64m | document_prepare | 5 | ms | 1589.470100 | 1535.696300 | 1608.251000 |
| 100-files-6.25m | search_scan_cached | 5 | ms | 27.298300 | 23.179200 | 37.045400 |
| 100-files-6.25m | search_cancel_latency | 5 | ms | 0.155300 | 0.110100 | 0.204800 |
| ansi-unicode-8m | terminal_parse | 5 | ms | 86.471500 | 82.435400 | 88.570700 |
| 24x80-screen | terminal_screen_snapshot_100 | 5 | ms | 0.297400 | 0.290200 | 0.603200 |

## 测试日志结果

| 日志 | 结果 | 通过 | 失败 | 忽略 |
| --- | --- | --- | --- | --- |
| fixtures.log | ok | 4 | 0 | 3 |
| native-exited_shell_launchers_do_not_hide_unknown_applications.log | ok | 1 | 0 | 0 |
| native-launcher_backed_shell_is_idle_then_detects_and_clears_programs.log | ok | 1 | 0 | 0 |
| native-running_application_is_detected_and_cleared_in_each_real_shell.log | ok | 1 | 0 | 0 |
| native-terminal-smoke.log | ok | 1 | 0 | 0 |
| tests-before.log | FAILED | 556 | 2 | 9 |
| tests-controlled-before.log | FAILED | 556 | 2 | 9 |
| tests-isolated-before.log | ok | 558 | 0 | 9 |
| tests-isolated.log | ok | 562 | 0 | 12 |
| tests-native.log | FAILED | 560 | 2 | 12 |
