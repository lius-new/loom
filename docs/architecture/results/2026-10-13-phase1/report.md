# Loom 阶段 1 执行与验收报告

更新日期：2026 年 10 月 13 日。项目：C:\Users\yys\loom。基础提交仍为 5ade2963ac1f2046b7ac54f6d2c58d5bf724bf45，阶段 0 和阶段 1 改动均保留在工作区，未提交。

阶段 1 的接线与渐进迁移边界已落地，本机自动验收通过。阶段 2 的搜索业务拆分尚未开始；阶段 0 的原生 IME、视觉/焦点、其他平台和安装包人工验收继续保留。

## 实施结果

| 任务 | 本轮实现 | 验收证据 |
| --- | --- | --- |
| R1.1 接线骨架 | main 只调用 app::run；窗口/图标启动在 app/bootstrap；根组件在 app/shell；原 input 接入 features/keymap；后台通知在 infrastructure/ui_dispatch；通知类型在 domain | 全目标编译、debug/release 完整测试、正式 release 构建、真实启动通过 |
| R1.2 状态归属与私有组合根 | app 的启动和 shell 实现不向功能/UI 公开；每个现存状态字段都有明确 owner/remove_in；禁止未登记新字段 | 118 个逐字段记录：117 个旧公开字段 + 1 个私有 Weak<AppRuntime> |
| R1.3 运行时资源 | FileLoader、ApplicationHandle、PollControl 和搜索取消令牌由独立 AppRuntime 持有；旧状态只弱引用运行时 | 快照不保活服务、运行时隔离、幂等清理和取消寿命测试；配置/会话兼容夹具通过 |
| R1.4 窄接口 | Toast UI 只收 Option<Toast> 和 dismiss(id)；读取/更新留在 app/presentation | 实际 UI 点击与旧卡片/旧回调保护测试通过；新根状态适配器触发门禁失败 |
| R1.5 架构门禁与可控测试工具 | 新反向依赖、根字段、runtime 桥接和过期例外均检查；UiTasks/ManualClock 已用于生命周期测试；CI 与基线脚本接入 | Node 的 7 项门禁自测通过；Rust 新增 6 项回归测试通过 |

没有拆分 Workspace 的跨文档/跨 pane 原子更新，也没有调整 undo/Vim/text 核心、Git 操作流程、搜索匹配/代次/debounce、终端协议或持久化格式。目录迁移中的 keymap 同名模块触发过一次 Clippy module_inception，已改名为 bindings 并保留兼容导出，最终 Clippy 无诊断，未增加 lint 豁免。

## 当前目录与依赖

详细说明见同目录的 架构边界说明.md 和 phase1-policy.json。源码现为 127 个 Rust 文件。

~~~text
main → app/bootstrap → app/shell
                       ├─ app/state（旧状态，归属已登记）
                       ├─ app/runtime（私有服务所有权）
                       ├─ app/presentation → ui/toast → domain/notification
                       ├─ features/keymap（原 input，保留桥接）
                       └─ infrastructure/ui_dispatch（原 background_ui）
model / editor / git / search / terminal 等旧实现按后续阶段迁移
~~~

已登记 92 个含旧依赖的源文件，覆盖生产代码与 cfg(test)。这些不是本轮新增的 92 个问题，而是既有接线的可追踪例外。每个条目都有依赖目标、迁移阶段和理由；新增文件不能继承旧文件的例外。AppState 的服务字段由 120 个公开字段中的 3 个移出，剩余公开字段后续按功能逐片私有化，当前仍保留旧桥接。

重要边界：Weak<AppRuntime> 虽不拥有服务，仍是一项暂存调用能力；AppState 尚不是纯业务 DTO。lgui State::get 仍克隆整体旧状态，Toast 的接口收窄不代表所有 clone 成本已解决。TerminalTabs 的旧 PTY 容器和 shell effect 的 worker 清理仍保持原结构，进一步分离在 R7。搜索取消令牌移动仅属于阶段 1 的资源归属整理，SearchState/commands/ports/service 的完整迁移留给 R2。

架构脚本是包含 cfg(test) 的词法门禁，识别 crate 根路径、分组 use 和受约束外部/标准库入口，排除注释和字符串。它不解析全部 Rust 类型别名、宏展开、路径绑定和循环；后续独立 crate 的编译边界仍需建立，不能把此检查视作全面类型级架构证明。

## 检查结果

| 检查 | 结果 |
| --- | --- |
| 架构门禁 | 127 个文件、118 个字段登记检查通过 |
| 门禁负例测试 | 7 通过，0 失败 |
| cargo fmt --all -- --check | 通过 |
| Clippy（保持现有 CI 参数） | 通过，0 诊断 |
| 默认 Git 环境 debug 完整测试 | 568 通过，0 失败，12 忽略 |
| 隔离 Git 环境 debug 完整测试 | 568 通过，0 失败，12 忽略 |
| 默认 Git 环境 release 完整测试 | 568 通过，0 失败，12 忽略 |
| Windows 四项真实 ConPTY smoke | 另行全部通过 |
| 正式 Loom release 构建 | 通过 |
| 隔离 APPDATA 的实际启动 | 运行 4 秒且根 effect 写入隔离 workspace.json，正常关闭，exit=0 |
| 最终 48 条检查/采样命令 | 48 条 exit=0 |
| 依赖文件和版本 | Cargo.toml/Cargo.lock hash 与阶段 0 相同 |
| 工作区差异检查 | git diff --check 通过，保留既有阶段 0 改动 |

默认完整测试从 562 项通过增加到 568 项通过，新增 6 项验证运行时寿命、隔离、取消、交付顺序、可重入投递及 Toast 点击适配。12 项 ignored 保持原登记：6 个性能、4 个 native smoke、1 个 native mouse 子进程辅助入口、1 个 SVG 人工观感检查；性能和 native smoke 已单独执行，不能对全部 ignored 无差别运行。

启动 smoke 为隐藏窗口子进程，仅对子进程 APPDATA 定向到本任务 work 中的临时目录；读取/保存均不使用用户真实 Loom 会话。只请求关闭本次创建的 PID 10292，无强制清理，也未关闭用户已有 Loom 进程。此 smoke 验证初始化和退出，不能替代系统原生 IME、视觉与焦点验收。记录在 startup-smoke.json。

正式应用 SHA256：07A9C71475A7B15A83C34FB9677313A3648B95ED8A937118B9386C48778D7F82。Cargo.lock：8E60A3117C5294406747E4964D3E69B2A31AB7E380F6E9ACD40AA0BB86DBCC0C；Cargo.toml：93B02658669CFC34FB072477F9C610D86A49C036F44662F32156EF56DEF63BB3。最终源码 hash 与采样开始时的 inventory.json 全部匹配。

## 性能比较与异常复核

六组性能测试均采用 release、一次预热和五次正式样本，得到 31 个指标、155 条记录，统计解析无 notices。完整结果在 性能比较.md、performance-comparison.json 和 performance.csv，原始日志保留。

历史中位数比较曾发现三项上升超过 10%：10 MiB 缓存读取 +15.70%，10 MiB 文档准备 +11.38%，搜索取消 +31.52%。其余 28 项未超过 10%。这些结果没有被删除或替换。

为判断是否持续退化，重建阶段 0 对照：全部 118 份 Rust 源码 hash 与阶段 0 修复后 inventory 完全一致；仅修改 scratch 包名/可执行文件名以隔离 Cargo 应用产物，依赖不变。对文件准备和搜索两组交错执行 A/B，每个二进制一次预热与五次正式样本，顺序每轮互换，总计 24 次成功执行、每版本 8 个指标/40 条记录。

| 首次异常指标 | 阶段 0 同环境对照中位数 | 阶段 1 同环境中位数 | 变化 |
| --- | ---: | ---: | ---: |
| ascii-10m/file_read_cached | 5.144400 ms | 4.965100 ms | -3.49% |
| ascii-10m/document_prepare | 226.957100 ms | 236.855400 ms | 4.36% |
| 100-files-6.25m/search_cancel_latency | 0.119400 ms | 0.070500 ms | -40.95% |

交错复测的 8 个指标中，最大上升为 1 MiB 文档准备 +9.65%，三项历史异常未复现持续超过 10% 的退化。本轮没有确认的持续性能回归，也不宣称性能优化；亚毫秒取消延迟和文件 I/O 的单次百分比受机器状态影响。A/B 原始日志、二进制 hash、逐源文件匹配记录及统计保存在 performance-ab。UI session 测量仍不能等同系统原生帧延迟、输入分位数或内存峰值。

## 剩余验收与下一阶段

原生 IME/候选窗口、视觉与系统焦点、macOS/Linux、远端 GitHub Actions 及安装包测试未在本轮执行，继续作为待验项；CI 文件已接入本地通过的门禁，但不能声称远端工作流运行成功。

R1.1–R1.5 的本地自动验收完成。下一阶段为 R2 搜索功能切片：建立私有 SearchModel 和请求/结果类型、端口与服务，收窄 UI 对根状态的访问，并覆盖可控的旧任务完成/取消/代次与 dirty buffer 导航场景。本轮停在阶段 1，不包含这些阶段 2 改动。
