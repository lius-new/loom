# R8 设置、快捷键与 chrome 迁移验收

2026-10-14：R8.1–R8.6 完成本机自动验收。工作目录 C:\Users\yys\loom，分支 refactor/r8-chrome；实现提交 9bdc9bf3a8f7f0e1efc4b3ae06ba5fb38dac6546。main 保持 R7 提交 0caaf0701f35a0b437523f0b06fa0b8550e8e9b9，未合并、未推送。

| 检查点 | 结果 |
| --- | --- |
| R8.1 | 设置、字体、主题 ID、JSONC 纯规则与文件/配置 watcher 适配拆分，资源归 UI |
| R8.2 | 设置/keymap 私有模型；输入、上下文事实、GUI 键事件、值帧与语义命令分离 |
| R8.3 | 文件任务、原生 watcher、文件/对话框与窗口服务归基础设施，app 保留跨模型协调；弱 lease 拒绝清理后交付 |
| R8.4 | welcome/home/title/status/tabs/menu/close/pane/diff 全部经值帧和语义命令，绘制与命中留 UI |
| R8.5 | 模型移除 runtime 引用和方法，显式 AppContext 保存模型句柄与 Weak<AppRuntime>；取消副作用归应用事务；旧根别名与 workspace 兼容导出删除 |
| R8.6 | 全量回归、所有权负例、冻结二进制性能对比、正常及无参数启动恢复和证据归档 |

架构门禁：272 个 Rust 源文件，根字段 30→15；R8 遗留例外 31→0、根 UI 适配器 12→0、旧根别名 0。通知/首页状态及 drawer 类型归 app/state/chrome，tabs 交互及 diff divider 归 UI。Workspace 仍在一次应用事务中更新。剩余框架 runtime 登记只用于 UiSession 测试/动画，不再提供模型服务访问。unicode_width 在单行文本测量中是精确登记的纯 UI 依赖。

默认 debug、独立 Git 配置 debug 和 release 三轮完整 workspace 均为 **715 通过、0 失败、15 ignored**（应用 618、编辑核心 68、独立域重复验证 29）。629 个原应用入口逐项映射、698 个原测试声明全部保留；702 个现有声明中新增 4 项，分别验证设置快照独立性、纯键事实与两个清理后排队交付竞态。忽略状态未改变。

59 项门禁/解析自测、fmt、all-targets、严格 CI Clippy、release 构建通过。Clippy 保持 CI 既有五项兼容 allowance，不新增豁免。4 项真实 ConPTY 回归通过，覆盖各 shell 往返、launcher 空闲/运行识别和退出后的进程探测。最终冻结 release 普通隔离启动和无参数恢复均自然退出，未强制清理；终端顺序、分组、宽度、用途、目录、选中项、可见性和高度 8 项持久化字段一致。

同机 Rust 1.98.0、冻结 before/after release 测试二进制，1 轮预热、10 轮交错正式样本，无并行构建：43 项指标（31 既有 + 12 生产终端）均完整采样。首轮唯一超过 10% 的指标为 Search 取消延迟，0.09055→0.1508 ms（+66.54%）；原始结果保留。确认该测量函数与底层 Search 源码逐字一致后，追加 30 对交错复测：0.14045→0.12125 ms（-13.67%），未持续复现。其余首轮时间指标最大上升 7.35%，12 项终端指标最大上升 3.34%。这些是测试场景的时间结果，不代表原生输入延迟、CPU 或内存改善。

证据见 [完整清单](results/2026-10-14-r8-chrome/evidence.json) 与 [43 项性能对比](results/2026-10-14-r8-chrome/performance.csv)。全部原始日志、两轮性能结果、启动恢复夹具、复验脚本及冻结二进制位于 target/baseline/2026-10-14-r8-final；各中间检查点在相应 results 和 target/baseline 目录保留。Cargo.toml、Cargo.lock、rust-toolchain.toml 相对 R8 起点不变。

本机实际使用 **Rust 1.98.0**；仓库固定 1.97.1 在本机不可用。固定工具链、人工视觉/IME/系统焦点、Linux/macOS、远端 CI 和真实安装/卸载仍待验。本报告仅确认本机自动验收结果。
