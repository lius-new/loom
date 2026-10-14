# R7 终端迁移最终本机验收

2026-10-14，分支 refactor/r7-terminal，起点 b414223822337bd61b840950cd94826921e10a72。R7.1–R7.7 本机自动验收完成，R8 未开始。此次授权完成 R7.5 effect/通知/保存、R7.6 窄终端/布局视图、R7.7 门禁、完整回归与性能复验；各步有独立本地检查点，无推送。

## 实现与行为

features/terminal 与 domain/application_layout 管理纯模型和规则。AppRuntime 唯一持有运行时注册资源，模型、应用绑定和 UI 端口通过 Weak 访问；移动保留同一 PTY，split 新建资源，关闭先断开模型/布局引用再处置，退休 SessionId 不重建。原有 8 项 JSON 字段、顺序、分组、宽度、目录和退出保存流程保持。

app/terminal_effects 管理恢复、profile 刷新、可见区域启动、探测、标签、blink、保存和 worker 清理。reader/waiter 核对 generation 并在生命周期锁内提交，回调在锁外；排队 UI 通知按 SessionId、generation、effect lease 再验，旧 mount cleanup 不停止新 mount。隐藏不杀进程，保存时读实时 cwd。

ui/terminal 只接收值帧、UI 状态投影和语义命令。app/terminal_view 组合端口，app/layout_map 持有场景/计划/settle；UI 保留绘制及框架事件事实。旧顶层终端和四个旧 UI 路径已删除，无兼容导出。模型、feature、infrastructure、UI 均无终端迁移例外；六项配置保留给 R8。

## 测试与门禁

默认 debug、进程级独立 Git 配置 debug、release 完整 workspace 均 **711 通过、0 失败、15 ignored**：应用 614、编辑核心 68、独立域重复验证 29。起点 673 通过；增加 25 项行为与 13 项域重复验证。原 14 ignored 不变，增加 1 项 opt-in 终端性能测试。603 个原应用入口逐项映射且结果一致；81 项源码测试移动，672→698 个声明（含平台 cfg），缺失 0。

49 Node 自测（架构 46、报告 3）、架构门禁、all-targets、fmt、CI 参数 Clippy、release 应用构建通过。239 Rust 文件，根/editor/workspace 30/30/9；31 项剩余例外均归 R8，12 个其他领域根 UI 适配器。收紧旧模块声明、嵌套资源和 grouped std 原生服务负例。新基准筛选与报告解析器已接入，脚本语法及 12 项指标解析已验证。

四项精确 ConPTY smoke 通过，覆盖实际 shell 往返、launcher 空闲/程序、替换退出与各 shell 前台程序清除。最终 release SHA-256：702A6B2094EDEB7FA24CD0D5D716A6D55C9DBFBAD49586E502B591526C8EE4B6。隔离正常启动 PID 19632 与无参数恢复 PID 1432 均正常退出 0，无强制清理；合成双终端 PowerShell/Cmd 的 8 项字段经保存文件复核一致。使用独立 APPDATA/LOCALAPPDATA，不修改用户配置、不关闭用户进程。真实 PTY 移动和快照保留时 shell/observer 退出由 R7.3 原生所有权回归覆盖；本次启动脚本不另宣称全面后代 PID 扫描。

## 同机性能

Rust 1.98.0，冻结 before/after release 测试二进制，共用测量体；固定输入/字体/屏幕/窗口，每版 1 轮预热、10 轮正式交错采样，无并行构建。原 31 项常规指标均无超过 10% 的中位数上升；完整样本、散布、节点数、源码清单与 hash 均保存。

终端专项 12 项如下，单位 ms；后缀 100/500 为每样本迭代次数，屏幕快照每次覆盖全部 n 会话，UI 使用生产 UiSession 构造。

| 会话数 | 指标 | before 中位数 | after 中位数 | 变化 | 节点 before/after |
| --- | --- | ---: | ---: | ---: | --- |
| 1 | production_screen_snapshot_100 | 4.54810 | 4.58610 | 0.84% | 0/0 |
| 1 | model_snapshot_500 | 0.04535 | 0.04130 | -8.93% | 0/0 |
| 1 | frame_projection_500 | 0.15610 | 0.24430 | 56.50% | 0/0 |
| 1 | terminal_ui_frame_100 | 69.48785 | 72.70290 | 4.63% | 163/163 |
| 8 | production_screen_snapshot_100 | 36.61685 | 37.26910 | 1.78% | 0/0 |
| 8 | model_snapshot_500 | 0.07050 | 0.08930 | 26.67% | 0/0 |
| 8 | frame_projection_500 | 0.58620 | 0.77740 | 32.62% | 0/0 |
| 8 | terminal_ui_frame_100 | 81.62050 | 83.25855 | 2.01% | 226/226 |
| 32 | production_screen_snapshot_100 | 147.44405 | 150.32325 | 1.95% | 0/0 |
| 32 | model_snapshot_500 | 0.23670 | 0.20695 | -12.57% | 0/0 |
| 32 | frame_projection_500 | 2.20500 | 2.64320 | 19.87% | 0/0 |
| 32 | terminal_ui_frame_100 | 139.76320 | 144.46620 | 3.36% | 442/442 |

屏幕快照约 +0.8%～+2%，UI 帧约 +2%～+4.6%，节点数一致。帧投影仍上升 +19.9%～+56.5%，每帧新增约 0.176/0.382/0.876 µs（1/8/32 会话）。来源是将旧控制器快照改为弱运行时查找、纯元数据和 DTO 构造；已消除重复标签读取，改为一次注册表批量查找，释放锁后读资源，再完成十轮复验。此开销为明确保留的架构取舍，不标为零回归。8 会话纯模型快照每次新增约 37.6 ns，聚合量很小，百分比不作改进/精确延迟结论。

第一次投影负载 before/after 字段不等价，列为诊断记录并排除最终结论；修正同为 9 字段 + 模型后诊断、批量查找优化后的最终样本全部保留。先前编译接线和门禁自测中的失败已修复，构建日志不抹除。基准是合成固定输入的生产路径，不等于原生输入延迟、窗口帧率、后台 CPU 或内存测试；原有 terminal_screen_snapshot_100 仍只测 screen.contents()。

## 验收清单

- [x] 纯类型/规则和布局域；PTY/shell/cwd/程序探测边界。
- [x] 纯会话模型、唯一运行时所有者、同 PTY 移动、split 独立、幂等 close/shutdown、弱快照不保活。
- [x] 按 SessionId/菜单目标协调命令、失效目标拒收、剪贴板失败保选区、profile blur 行为。
- [x] 输出/退出/探测/通知 generation 与取消保护、旧 mount cleanup 隔离、可见区域启动。
- [x] 终端和布局窄视图；删除旧路径、终端无迁移例外；旧测试逐项映射。
- [x] 默认/独立 Git/release 全量回归、工程门禁、4 原生 smoke、隔离启动与无参数 JSON 恢复。
- [x] 同机冻结性能 A/B、超过 10% 的投影开销调查和复验、完整原始数据保留。
- [ ] 固定 Rust 1.97.1（本机不可用；实际为 1.98.0，Cargo/lock/toolchain 文件未改）。
- [ ] 原生人工 IME/视觉/焦点/剪贴板/TUI/拖放全矩阵；Linux/macOS、远端 CI。
- [ ] staged/portable/installer 和真实安装/卸载；另行原生延迟/CPU/内存测量。

原始记录位于 target/baseline/2026-10-14-r7-terminal；结构化汇总见 validation.json、test-migration.json、source-audit.json、source-manifest.json 和 performance-comparison.json/csv。各步历史结果保留，不将本机自动通过扩大为全部平台验收。
