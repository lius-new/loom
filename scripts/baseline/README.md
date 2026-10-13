# 阶段零复跑脚本

在 Windows PowerShell 7 中从项目根运行：

~~~powershell
.\scripts\baseline\run.ps1 -Offline -Samples 5
~~~

项目工具链由 rust-toolchain.toml 固定。Windows 上脚本会优先使用 SHA-256 校验通过的本地 Skia 预编译包，避免 Cargo 离线模式下 Skia 构建脚本仍尝试联网。也可用 -SkiaArchive 显式指定归档；路径、版本或校验错误会立即失败。环境变量在脚本结束时恢复，详见 [构建说明](../build/README.md)。跨机器或工具链变化后的性能记录应建立新基线，不直接用旧机器数据判定回归。

正式 release 构建通过后，可执行隔离启动检查（只启动并关闭本脚本创建的进程；APPDATA 与 LOCALAPPDATA 均指向新建的测试目录）：

~~~powershell
.\scripts\baseline\startup-smoke.ps1 -OutputDirectory target/baseline/new-startup-check
~~~

该检查验证启动存活、正常关闭和 synthetic 工作区会话保存，不替代原生 IME、视觉或系统焦点的人工验收。

默认生成 target/baseline/时间戳目录，也可通过 -OutputDirectory 指定一个新的目录。已有日志目录会被拒绝，以免混入不同源码版本的结果。使用记录中相同的 Rust、Clippy、Git、Node 和机器条件，才可与当前性能基线直接比较。

脚本先执行阶段 1 架构门禁及其负例自测（需要 Node），再执行格式、Clippy、默认环境 debug/release 完整测试与正式 release 应用构建，再使用进程级独立 Git 配置复测；不修改用户/系统 Git 配置。继续执行四项 Windows native smoke，以及三项现有和三项新增性能测试。性能使用 release，每组样本零为预热，随后五次正式样本。任何检查失败时继续收集结果并最终退出非零，不能视为全绿。

- -PerformanceSet All 为默认；Legacy、Additional、None 分别选择原有性能、新增性能或只执行检查。
- -SkipChecks 只采集指定的性能组。
- run.ps1 执行检查并记录环境；inventory.ps1 收集字段、hash 和词法依赖候选；report.mjs 生成测试清单、Clippy 定位和统计。
- 完整回归与 Clippy 使用 --workspace，包含独立编辑核心；inventory 同时记录 src 与 crates/editor-core。report 汇总各 crate 的 unit/integration/doc-test 结果，末尾的空 doctest 不会覆盖通过数量或隐藏前面的失败。
- R4 增加 crates/workspace-tests 无 GUI 域验证入口，复用生产 domain 源码；测试以 isolated::domain 前缀区分。全 workspace 包含该入口的重复规则验证，报告需单列其通过数量，不能当作新增独立行为覆盖。
- R3 应用事务迁移后，原有编辑和 UI 性能测试分别位于 app::editor_baseline_tests::perf_baseline 和 app::editor::tests::large_file_interaction_baseline；run.ps1 已同步精确筛选名称，指标与样本规则保持一致。
- 原生终端 smoke 只在 Windows 执行。不要对所有 ignored 测试盲目使用全局 --ignored；native_mouse_probe 是辅助入口。

重新生成已有日志的统计：

~~~powershell
node scripts/baseline/report.mjs 'C:\path\baseline'
~~~

完整报告见 docs/baseline/README.md，行为矩阵见 docs/baseline/behavior-matrix.md。初始结果位于 docs/baseline/results/2026-10-13；检查修复后的结果位于 docs/baseline/results/2026-10-13-fixed。原始日志保存在输出材料中，不复制进源码结果目录。
