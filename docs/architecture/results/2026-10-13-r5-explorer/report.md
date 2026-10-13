# R5 Explorer 迁移验收

2026-10-13，基础提交 a29abb5（R4 检查点），分支 `refactor/r5-explorer`。R5.1–R5.6 本机
自动验收完成。结构见 [Explorer 迁移](../../explorer-migration.md)，逐步记录见
[迁移方案](../../explorer-migration-plan.md)。

## 提交

| 步骤 | 提交 | 内容 |
| --- | --- | --- |
| R5.1 | a5abed2 | 类型、端口、纯规则与文件系统适配 |
| R5.2 | 3fe274e | ExplorerModel/UI 状态与抽屉 chrome 归属，删除 file_tree |
| R5.3 | d803b6f | Explorer 命令与跨功能事务迁入 app/explorer |
| R5.4 | f33deb8 | watcher 竞态测试与准备阶段 A/B |
| R5.5 | a670009 | 视图与右键菜单改为投影 + 语义命令 |
| R5.6 | 本提交 | 删除根状态类型别名、关闭 R5 登记、门禁与文档 |

## 指标

| 项 | R4 | R5 |
| --- | --- | --- |
| 根字段登记 | 98 | 76 |
| legacy exception 文件 | 60 | 57 |
| 根状态 UI 适配器 | 25 | 23 |
| `workspace_actions.rs` 行数 | 2429 | 约 1000 |
| 门禁自测 | 18 | 23 |
| 完整 workspace 测试 | 615 通过 / 12 忽略 | 650 通过 / 13 忽略 |

测试 650 = 应用 566 + 编辑核心 68 + 独立域验证 16；新增的忽略项是
`prepare_batch_cost_benchmark` 基准。迁移的测试保留原名（共 36 项）：file_tree 4（3 项进
feature，1 项进基础设施）、workspace_actions 中 Explorer 相关 20（6 项命名校验/最近目录进
feature，Windows shell 路径 1 项进基础设施，13 项进 app/explorer_tests）、sidebar 5、
context_menu 7。新增 35 项：feature 内存端口 17、基础设施 2、app 命令 4、watcher 竞态 6、
视图命令 6；与 R4 的 615 相差正好 35。

## 本机检查

| 检查 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| CI 参数 Clippy（全部目标，`-D warnings`） | 通过，0 诊断 |
| `cargo test --locked --workspace` | 650 通过，0 失败，13 忽略 |
| `cargo test -p loom-workspace-tests` | 16 通过 |
| 门禁与自测 | 通过；23/23 |
| release 构建 | 通过 |
| 隔离启动/恢复 smoke（Windows PowerShell 5.1，debug 每步 + release 收尾） | 带参数打开工作区、正常关闭写会话、无参数重启恢复根与最近目录；exit 0，无强制清理 |

## 性能

`prepare_batch` 冻结 release 测试二进制交错 A/B（R5.1 克隆整个 AppState 对当前代码只克隆
Explorer 模型），各 5 轮、每轮 3 次预热 + 30 样本：单项变更中位数 3.22–3.48 ms 对
3.22–3.53 ms，全量重扫 17.66–20.46 ms 对 17.57–18.57 ms，无可测差异；克隆本身 13.8 µs
对 8.6 µs。原始数据见 [prepare-batch-ab.txt](prepare-batch-ab.txt)。不声称性能提升。

## 未执行与限制

- 31 项常规性能基线与 `startup-smoke.ps1` 需要 PowerShell 7，本机未安装，未运行；以 5.1
  等价脚本替代启动/恢复验证。应在具备 PowerShell 7 的机器上补跑 `scripts/baseline/run.ps1`。
- 原生交互（树点击/展开、拖放到 pane、行内输入 IME、右键菜单视觉、回收站、文件剪贴板、
  在资源管理器中显示）没有人工验收；自动测试覆盖命令与状态，不代表像素级或系统集成结果。
- macOS/Linux、远端 CI、安装包验收仍按阶段 0 保留为待验项。
- `State::get` 仍克隆完整根值；视图投影收窄的是接口而非读取成本。
