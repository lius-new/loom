# R3 应用编辑事务、交互状态与文本 UI 验收

2026-10-13。本轮接续 R3.1，完成阶段 3 的应用编辑事务与文本 UI 边界迁移。
阶段 4–8 尚未开始。验收时改动均在工作区；本轮纳入 R3 检查点提交。

## 结果

- 普通编辑、剪贴板、IME、指针选择、撤销与滚动事务迁至 app/editor.rs；Vim Host 编排迁至 app/editor_vim.rs。
- 20 个根字段与原 10 个交互字段组成 app/state/editor.rs 的 EditorState；根字段从 118 降到 98，子状态 30 项继续逐一登记。
- 文本视图、光标、行号、语法、菜单和输入框按键转换归入 ui/editor。视图接收 pane 投影与事件回调，不接收 AppState、Workspace 或完整 Vim。
- Workspace 保留一次 State 更新，继续原子修改共享文档与关联 pane 选区；未替换文本算法或拆 reactive store。
- Language 与 LineChange 采用纯域类型；单行输入框直接复用核心。legacy exception 文件从 76 降到 64，逐项清理过期登记。
- Git hunk peek 由 app 注入覆盖层；其加载与操作仍为 R6 适配器。Diff、标签和窗格等旧 UI 接线继续按 policy 登记迁移。

结构与约束见 [编辑器迁移说明](../../editor-migration.md)。

## 自动验证

49 项 baseline 检查全部 exit 0，包括格式、严格 Clippy、门禁及 14 项门禁自测、
3 项报告自测、debug/release/隔离 Git 配置完整回归、release 构建、四项 Windows ConPTY
与六组性能采样。三轮完整 workspace 回归均为 591 通过、0 失败、12 忽略。

相对 R3.1 的 597 条测试清单（含 12 项 ignored），按目录迁移映射后全部保留；
现为 603 条，新增 6 项：非活动 pane 快照、过期指针拒收、晚到 IME 原目标与撤销、
Blur 分组、修改标记投影范围、真实菜单点击与编辑焦点。映射见 [test-migration.json](test-migration.json)。

新的独立 target 在故意无效的 Skia URL 与 LLVM 路径下运行核心，68 项通过。
核心依赖树只有 regex/Unicode，未编译 lgui/Skia。独立核心源码没有进一步改变。
正式应用在独立 APPDATA/LOCALAPPDATA 的 synthetic 工作区存活、保存会话并正常关闭；
退出码 0，无强制清理。原生手工 IME/视觉、跨平台、远端 CI 和真实安装/卸载仍待验。

## 同机性能

每组一次预热、五次正式 release 样本，共 31 指标/155 记录。相对 R3.1 同机最终记录，
9 项 UI 指标中位数变化 -2.51% 至 +5.79%。
这些 UiSession/微基准不能代表原生帧延迟或内存峰值，也不能证明根克隆成本已降低。

超过 +10% 的历史比值保留如下，未隐藏或重新挑选样本：

| 场景 | 指标 | 前值 | 当前值 | 变化 |
| --- | --- | --- | --- | --- |
| 100-files-6.25m | search_cancel_latency | 0.106800 ms | 0.146600 ms | +37.27% |

搜索取消是短时延线程采样；本轮未修改搜索扫描/取消算法，不按该百分比声称确定回归或优化。

全量对照见 [validation.json](validation.json)、[performance.csv](performance.csv)，
原始日志在 [本机输出](../../../../target/baseline/2026-10-13-r3-transactions-accepted/measurements.md)。
首次验收亦保留在 target/baseline/2026-10-13-r3-transactions；随后迁移阶段零测试目录、
清理输入框兼容导出后，以本报告所列新目录再次完整验收，不混合两次源码的性能日志。

## 下一步

阶段 3 本机自动验收完成。下一步 R4 整理 Workspace/Document/Pane 域与文件/会话适配，
保持共享文档、跨 pane 选区及 Undo 的一次原子更新。State::get 仍克隆整个根值再投影，
本轮没有解决该开销；其优化需独立证据与验收。
