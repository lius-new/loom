# R4 工作区域与文件、会话边界验收

2026-10-13，基础提交 4895cc9。R4 实现与本机自动验收完成，本轮纳入 R4 检查点提交。
下一步 R5；R5–R8 尚未开始。

## 已完成

- 共享文档、pane/tab/view、preview、dirty、undo、磁盘冲突/安装规则归入 domain/workspace；metadata 与 pane layout 分别迁入 domain/document、domain/pane_layout。
- 域接收明确的 DiskRead/DiskState，不执行文件 I/O；纯 Rect 与语言元数据不依赖 lgui 或主题。
- app/workspace 保留一次根 State 更新，组合只读载荷；Git 请求/刷新/行高在 app/workspace_git，后台回收在应用事务入口。
- 稳定 editor 会话 DTO 与树快照归入域层；会话 envelope、保存/恢复协调与原恢复任务归入 app；JSON、路径和原子写入归入基础设施。
- 新增 workspace-tests 验证包，用同一份生产 domain 源码建立无 GUI/app/Git 的测试入口，并加入 CI 任务。
- 根状态 98、editor 30 项登记保持；workspace 核心 9 项逐字段登记；legacy exception 文件从 64 减为 60。

结构、事务约束和兼容路径见 [工作区域迁移](../../workspace-migration.md)。

## 回归与独立验证

49 项 baseline 检查全部 exit 0，包括格式、严格 Clippy、门禁、18 项门禁自测与 3 项报告
自测、debug/release/隔离 Git 配置完整回归、release 构建、四项 ConPTY 和六组性能采样。
三轮完整 workspace 回归均为 615 通过、0 失败、12 忽略。其中应用 531、编辑核心 68、
独立域验证 16；后者复测同一份域规则，不能把它当作 16 项新增独立行为。

按目录映射，R3 的 603 条测试清单（含 ignored）全部保留，现为 627 条。新增 8 项行为
用例：Unicode 跨 pane 编辑/撤销、注入磁盘冲突与接受、缺失/失败/恢复区别、关闭占位
与晚到准备、重命名保留身份/视图/历史、pending 会话插入顺序、空 pane 剔除与恢复、
preview 复用/移动后的回收队列清空。清单见 [test-migration.json](test-migration.json)。

新的独立 target 在故意无效的 Skia URL 和 LLVM 路径下编译并通过 16 项域测试，依赖树
只有编辑核心、serde、serde_json 及其子依赖，未编译 GUI/app/Git。域门禁拒绝 I/O/GUI/
反向依赖和迁移例外，验证入口同时限制源码路径和依赖清单。

正式应用在独立 APPDATA/LOCALAPPDATA 的 synthetic 工作区启动、保存会话、正常关闭，
退出码 0，无强制清理。原生人工、macOS/Linux、远端 CI 和真实安装/卸载仍待验。

## 性能与调查

常规 release 采样每组一次预热和五次正式样本，31 指标/155 记录。相对 R3 同机最终记录，
9 项 UI 中位数变化 -2.58% 至 +4.28%。
原始超过 +10% 的历史比值完整保留：

| 场景 | 指标 | 前值 | 当前值 | 变化 |
| --- | --- | --- | --- | --- |
| ascii-1m | typing_per_char | 0.020144 ms | 0.022188 ms | +10.15% |
| ascii-1m | undo_paste | 0.416700 ms | 0.468800 ms | +12.50% |

因此从已提交 R3 源码构建前版本，并冻结前/后 release 测试二进制做交错对照。首次五次
样本中 10 MiB undo 出现 +20.92% 信号；后续一轮因部分采样与重建重叠，明确排除，不用于
结论。最后在编译完成后，一次预热与每版本十次样本、交替执行顺序，八项编辑时延变化
-5.44% 至 +4.73%，历史内存的舍入值一致；未确认持续编辑回归。
原始历史比值和首次信号均未覆盖，不能声称所有采样低于 10% 或本轮提升了性能。
未为采样结果调整存储算法或撤销规则。

这些微基准/UiSession 数据不代表原生帧延迟或内存峰值；State::get 仍克隆完整根值。
对照二进制、sha256 与所有日志保留在 target/baseline/2026-10-13-r4-edit-ab*；当前测试
二进制已重建并核对 543 条应用测试入口，独立验证包另计。

全量数据：[validation.json](validation.json)、[performance.csv](performance.csv)。
原始本机输出：[measurements.md](../../../../target/baseline/2026-10-13-r4-workspace/measurements.md)。

## 接续范围

下一步 R5 迁移 Explorer 的私有状态、目录/文件命令与 UI 窄接口。继续保留文件 ID、
共享文档、跨 pane 选区及根更新的事务边界。Git 只读实现、application layout 和旧 UI
适配器按 R6–R8 继续迁移；本轮不声称整个 model 或所有文件/GUI 接线已完成纯化。
