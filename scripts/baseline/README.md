# 阶段零复跑脚本

在 Windows PowerShell 7 中从项目根运行：

~~~powershell
.\scripts\baseline\run.ps1 -Offline -Samples 5
~~~

默认生成 target/baseline/时间戳目录，也可通过 -OutputDirectory 指定一个新的目录。已有日志目录会被拒绝，以免混入不同源码版本的结果。使用记录中相同的 Rust、Clippy、Git、Node 和机器条件，才可与当前性能基线直接比较。

脚本先执行阶段 1 架构门禁及其负例自测（需要 Node），再执行格式、Clippy、默认环境 debug/release 完整测试与正式 release 应用构建，再使用进程级独立 Git 配置复测；不修改用户/系统 Git 配置。继续执行四项 Windows native smoke，以及三项现有和三项新增性能测试。性能使用 release，每组样本零为预热，随后五次正式样本。任何检查失败时继续收集结果并最终退出非零，不能视为全绿。

- -PerformanceSet All 为默认；Legacy、Additional、None 分别选择原有性能、新增性能或只执行检查。
- -SkipChecks 只采集指定的性能组。
- run.ps1 执行检查并记录环境；inventory.ps1 收集字段、hash 和词法依赖候选；report.mjs 生成测试清单、Clippy 定位和统计。
- 原生终端 smoke 只在 Windows 执行。不要对所有 ignored 测试盲目使用全局 --ignored；native_mouse_probe 是辅助入口。

重新生成已有日志的统计：

~~~powershell
node scripts/baseline/report.mjs 'C:\path\baseline'
~~~

完整报告见 docs/baseline/README.md，行为矩阵见 docs/baseline/behavior-matrix.md。初始结果位于 docs/baseline/results/2026-10-13；检查修复后的结果位于 docs/baseline/results/2026-10-13-fixed。原始日志保存在输出材料中，不复制进源码结果目录。
