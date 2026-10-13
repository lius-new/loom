# 阶段零自动采样结果

样本零为预热，已排除。以下为同机 release 测试中位数和范围；UI session 数据不等于原生窗口帧率。

| 场景 | 指标 | 样本 | 单位 | 中位数 | 最小值 | 最大值 |
| --- | --- | --- | --- | --- | --- | --- |
| ascii-1m | file_read_cached | 5 | ms | 0.635600 | 0.610700 | 0.695200 |
| ascii-1m | document_prepare | 5 | ms | 22.933300 | 22.322600 | 28.161100 |
| ascii-10m | file_read_cached | 5 | ms | 4.993400 | 4.815200 | 5.772300 |
| ascii-10m | document_prepare | 5 | ms | 230.228600 | 223.798200 | 237.476100 |
| ascii-64m | file_read_cached | 5 | ms | 36.629500 | 36.091100 | 46.858200 |
| ascii-64m | document_prepare | 5 | ms | 1517.556900 | 1463.888000 | 1548.030500 |
| ascii-1m | typing_per_char | 5 | ms | 0.024001 | 0.023545 | 0.024744 |
| ascii-1m | paste_1m | 5 | ms | 26.068900 | 25.590200 | 27.051900 |
| ascii-1m | undo_paste | 5 | ms | 0.320900 | 0.280600 | 0.412900 |
| ascii-1m | select_all_indent | 5 | ms | 26.497600 | 26.044500 | 28.274400 |
| ascii-1m | history_bytes_rounded | 5 | bytes | 0.000000 | 0.000000 | 0.000000 |
| ascii-10m | typing_per_char | 5 | ms | 0.168250 | 0.155516 | 0.171162 |
| ascii-10m | paste_1m | 5 | ms | 27.072100 | 26.757500 | 27.998500 |
| ascii-10m | undo_paste | 5 | ms | 0.760200 | 0.737000 | 0.827600 |
| ascii-10m | select_all_indent | 5 | ms | 285.930500 | 262.558900 | 316.165400 |
| ascii-10m | history_bytes_rounded | 5 | bytes | 314572.800000 | 314572.800000 | 314572.800000 |
| 100-files-6.25m | search_scan_cached | 5 | ms | 25.650200 | 24.694900 | 27.005600 |
| 100-files-6.25m | search_cancel_latency | 5 | ms | 0.137700 | 0.091300 | 0.209500 |
| 100-tabs | pointer_update | 5 | ms | 0.066111 | 0.061962 | 0.067479 |
| 500-tabs | pointer_update | 5 | ms | 0.400219 | 0.377856 | 0.402300 |
| ansi-unicode-8m | terminal_parse | 5 | ms | 86.965300 | 84.954400 | 97.044500 |
| 24x80-screen | terminal_screen_snapshot_100 | 5 | ms | 0.300800 | 0.296800 | 0.323900 |
| 8 MiB single line | ui_document_prepare | 5 | ms | 203.259800 | 200.541600 | 222.270200 |
| 8 MiB single line | ui_100_viewports_and_clicks | 5 | ms | 15.958100 | 15.251900 | 16.970200 |
| 8 MiB single line | ui_20_edits_and_renders | 5 | ms | 5.237200 | 4.853100 | 5.519000 |
| Unicode single line | ui_document_prepare | 5 | ms | 81.566200 | 79.033400 | 83.021200 |
| Unicode single line | ui_100_viewports_and_clicks | 5 | ms | 14.111200 | 13.664400 | 16.772700 |
| Unicode single line | ui_20_edits_and_renders | 5 | ms | 4.573900 | 4.170000 | 5.100900 |
| many short lines | ui_document_prepare | 5 | ms | 380.456500 | 378.511700 | 382.867800 |
| many short lines | ui_100_viewports_and_clicks | 5 | ms | 35.230200 | 31.618400 | 37.353500 |
| many short lines | ui_20_edits_and_renders | 5 | ms | 14.039900 | 12.967500 | 14.787200 |

## 测试日志结果

| 日志 | 结果 | 通过 | 失败 | 忽略 |
| --- | --- | --- | --- | --- |
| native-bash-replacement-smoke.log | ok | 1 | 0 | 0 |
| native-launcher-smoke.log | ok | 1 | 0 | 0 |
| native-program-name-smoke.log | ok | 1 | 0 | 0 |
| native-terminal-smoke.log | ok | 1 | 0 | 0 |
| tests-isolated.log | ok | 562 | 0 | 12 |
| tests-native.log | ok | 562 | 0 | 12 |
| tests-release.log | ok | 562 | 0 | 12 |
