# Loom 阶段零基线报告

> 当前进度：阶段 2 本机自动验收已完成，582 项默认测试通过；见 [阶段 2 收尾报告](../architecture/results/2026-10-13-phase2/report.md) 和当前进度文档。下文保留该阶段的历史截止记录。

> 后续进度：阶段 1 的本机自动验收已完成，见 [阶段 1 执行报告](../architecture/results/2026-10-13-phase1/report.md)。本文保留阶段 0 截止状态，历史数据不改写。当前默认完整测试为 568 项通过、0 项失败、12 项忽略。

更新日期为 2026 年 10 月 13 日，基础提交为 5ade2963ac1f2046b7ac54f6d2c58d5bf724bf45，工作区包含阶段零资产与本轮检查修复，尚未提交。阶段零已建立本机可复跑的测试、兼容夹具、性能采样和依赖记录；本轮仅修复已登记的工程检查与测试夹具问题，未进入阶段一，未改变业务行为、目录架构、依赖版本或持久化格式。

格式检查和按现有 CI 参数运行的 Clippy 均已通过，原先 11 处诊断全部消除。默认系统 Git 配置仍为 core.autocrlf=true，但两项换行敏感测试现在通过；默认环境和独立 Git 配置中的完整测试均为 562 项通过、0 项失败、12 项忽略。四项真实 ConPTY smoke 另行执行通过，六组性能测试完成预热与五次正式采样。

没有新增 lint 豁免、禁用测试或放宽断言。原始失败记录与初始性能基线保留，本轮结果另存于 results/2026-10-13-fixed，初始结果仍在 results/2026-10-13。

## 本轮交付

| 内容 | 位置或用途 |
| --- | --- |
| 本报告 | 检查结果、性能摘要、已有问题和阶段零验收状态 |
| 行为矩阵 | 当前需要保护的行为、测试入口和人工验证边界 |
| 四份 synthetic 夹具 | 旧会话、分屏会话、设置 JSONC、快捷键 JSONC |
| 四项普通夹具测试 | 对原有读取和解析接口验证兼容，不写用户配置 |
| 三项 opt-in 性能测试 | 1/10/64 MiB 文件读取与准备、搜索扫描/取消、终端解析 |
| run.ps1 | 按现有 CI 参数复跑检查、两类 Git 环境、native smoke 和采样 |
| inventory.ps1 | 当前字段、源码 hash 和六类依赖候选记录 |
| report.mjs | 从原始日志生成逐测试清单、Clippy 定位和性能统计 |
| 原始日志与机器数据 | environment、checks、inventory、test-inventory、measurements 和 CSV |

阶段零初始交付新增 docs/baseline、scripts/baseline 和 src/editor/phase0_baseline.rs，并在 src/editor/mod.rs 增加 cfg(test) 接线。本轮在 12 个原有源码文件内做局部修复：格式、两个私有类型别名、unused re-export、无必要的字符串/测试克隆、等价完整分组迭代、仅用于测试的 Git 仓库配置，以及 release 测试的控制台构建条件。AppState 字段、任务机制、模块归属、业务入口和持久化格式不变。Cargo.toml、Cargo.lock 的 SHA-256 不变。

## 环境与可复现条件

| 项目 | 本次实测环境 |
| --- | --- |
| 系统 | Windows 11 专业版，10.0.26200，x64 |
| CPU | AMD Ryzen 5 5600GT，6 核 12 逻辑处理器 |
| 物理内存 | 系统报告 33,605,750,784 字节，约 31.3 GiB |
| rustc | 1.98.0，88d9e12ae，x86_64-pc-windows-msvc |
| Cargo | 1.98.0，797e8a9bc |
| rustfmt | 1.9.0-stable |
| Clippy | 0.1.98 |
| Git | 2.55.0.windows.4，系统 Git |
| Node | v24.19.0 |
| lgui | Cargo.lock 锁定 46e13848aefd9fd92f3370f111a5a0216c811438 |
| 执行方式 | 使用现有 target 缓存，Cargo --locked --offline，release 性能测量 |
| 性能采样 | 每组一次预热加五次正式执行，测试线程数为 1 |

每个结果目录保存其对应的 environment.json 与 inventory.json，初始采集另保留 environment-before.json 和 source-before.json。本轮记录源码 hash、工作区状态及依赖 hash，用于区分未提交的两个基线版本。执行环境未提供 rustup，因此未新增可能触发安装的 rust-toolchain.toml；现有 CI 仍使用 stable，工具链固定作为独立工程任务处理。

本轮没有修改系统、用户或 Loom 项目的 Git 配置。测试共享 helper 只在新建临时测试仓库中设置 core.autocrlf=false、core.eol=lf，使 synthetic 夹具的精确 LF 断言独立于宿主默认值。产品 GitCommandRunner 及真实用户仓库的换行策略完全保持原样。

复跑脚本仍分别执行默认环境和独立 Git 配置测试。独立配置使用进程级 GIT_CONFIG_NOSYSTEM=1 与 GIT_CONFIG_GLOBAL；现在默认环境也通过，因此通过结果不再依赖该隔离方式。

最初仅使用 GIT_CONFIG_COUNT 注入 autocrlf 的复测仍然失败，因为现有 GitCommandRunner 在启动 Git 时自行设置该变量。后续采用独立配置文件完成原因验证，此次失败也保留在 tests-controlled-before.log。

## 检查结果

| 检查 | 阶段零修复前 | 本轮修复后 | 结论 |
| --- | --- | --- | --- |
| 格式检查 | 退出 1 | 退出 0 | 三处排版差异消除 |
| Clippy，现有 CI 参数 | 退出 101，11 个不同位置 | 退出 0，零诊断 | 参数与豁免列表未放宽 |
| 默认 Git 环境完整测试 | 560 通过，2 失败，12 忽略 | 562 通过，0 失败，12 忽略 | 宿主 autocrlf=true，两个原失败测试通过 |
| 独立 Git 配置完整测试 | 562 通过，0 失败，12 忽略 | 562 通过，0 失败，12 忽略 | 574 个当前 Windows 注册测试，数量不变 |
| release 完整测试 | 初始未执行全量 release 测试 | 562 通过，0 失败，12 忽略 | ConPTY mouse helper 的测试构建条件已修复 |
| 正式 release 应用构建 | 初始基线未执行 | 构建通过，PE subsystem=2 | 正式程序仍为 Windows GUI；测试二进制 subsystem=3 |
| 四个阶段零兼容夹具 | 已通过 | 在完整测试中全部通过 | 夹具与解析行为未修改 |
| 真实 shell round-trip | 默认忽略 | 单独执行 1 通过 | PowerShell、Cmd、Bash 路径通过 |
| launcher-backed ConPTY 检测 | 默认忽略 | 单独执行 1 通过 | 闲置、程序检测和清除通过 |
| Git Bash 进程替换 | 默认忽略 | 单独执行 1 通过 | launcher 退出后应用仍可检测 |
| 各真实 shell 程序名检测 | 默认忽略 | 单独执行 1 通过 | ping 程序名出现和退出清除通过 |
| 性能测试 | 六组均已通过 | 六组重新执行 | 31 项指标，每项五个有效正式样本 |
| 差异检查 | 阶段零资产未提交 | git diff --check 通过 | 尚未提交，未做阶段一迁移 |

格式命令为 cargo fmt --all -- --check。

Clippy 命令与现有 CI 参数一致：

~~~text
cargo clippy --locked --offline --all-targets -- -D warnings
  -A clippy::redundant_closure_call
  -A clippy::collapsible_if
  -A clippy::too_many_arguments
  -A clippy::let_and_return
  -A clippy::manual_div_ceil
~~~

这里是 Windows 上按同样参数运行的结果，不代表 Linux CI 已重新运行。普通测试均使用 cargo test --locked --offline -- --nocapture。ChangeSet 类型和内部用法保留，只移除了无人使用的模块 re-export；对应编译告警已消除。

默认完整测试中的 12 个 ignored 分类：六个性能测试均已单独采样，四个终端 smoke 均已单独执行；native_mouse_probe 是由正常 wheel 测试启动的辅助入口，不单独运行；SVG 标签外观检查仍需人工视觉验收。ignored 并不等于失败，也不等于这些入口全部未经执行。

## 初始性能基线

以下保留阶段零修复前五次正式执行的中位数，完整数据位于 results/2026-10-13。修复后的五次复测与对比另存于 results/2026-10-13-fixed；每个版本的 performance.csv 与 measurements.json 均保持对应关系。不同机器、工具链或输入不可直接比较。

### 普通编辑

| 文档 | 连续输入每字符 | 粘贴约 1 MiB | 撤销粘贴 | 全选并缩进 |
| --- | --- | --- | --- | --- |
| 约 1 MiB ASCII 多行 | 24.783 µs | 29.033 ms | 0.301 ms | 28.996 ms |
| 约 10 MiB ASCII 多行 | 160.189 µs | 29.334 ms | 0.717 ms | 289.360 ms |

复用现有 perf_baseline：通过当前 editor_view 入口执行 200 次中部输入，之后粘贴、撤销、全选缩进。输入按当前行为合并撤销步骤。历史日志为一位小数 MiB，1 MiB 样例输出 0.0 MiB 不表示完全没有历史分配，10 MiB 样例为 0.3 MiB；这不是进程内存峰值。

现有编辑核心文档中的旧性能值属于此前提交和测量环境。本次只建立新的同机基线，不依据两份数值差异直接认定性能回归。

### 视图与标签

| 场景 | 文档准备 | 100 次 viewport render 与命中 | 20 次尾部编辑与 render |
| --- | --- | --- | --- |
| 8 MiB ASCII 单长行 | 209.476 ms | 15.800 ms | 5.208 ms |
| Unicode 单长行 | 82.591 ms | 13.556 ms | 4.387 ms |
| 500,000 条短行 | 412.510 ms | 35.431 ms | 13.782 ms |

100 个标签的 pointer 更新平均值中位数为 60.629 µs；500 个标签为 383.356 µs。它们都是现有 UI session/状态接线的测量，不能当作原生窗口帧率、Skia 光栅化耗时或端到端输入延迟分位数。

### 文件 搜索与终端

| 场景 | 指标 | 中位数 |
| --- | --- | --- |
| 1 MiB ASCII 文件 | cached UTF-8 读取 / 文档准备 | 0.725 ms / 24.532 ms |
| 10 MiB ASCII 文件 | cached UTF-8 读取 / 文档准备 | 5.116 ms / 244.790 ms |
| 64 MiB ASCII 文件 | cached UTF-8 读取 / 文档准备 | 37.753 ms / 1,589.470 ms |
| 100 个文件，共 6.25 MiB，100 个匹配 | cached 搜索扫描 | 27.298 ms |
| 同一搜索输入，扫描开始后请求取消 | 取消请求到搜索返回 | 0.155 ms |
| 约 8 MiB ANSI 和 Unicode 输出 | vt100 解析 | 86.472 ms |
| 24×80 终端屏幕 | 100 次文本快照 | 0.297 ms |

文件测量先写 synthetic 文件并预热缓存，文件生成不计入耗时；读取和 PreparedTextFile 构建分别计时，后者包括当前索引和磁盘状态准备。64 MiB 为现有编辑大小上限。数据不是冷盘性能，也不是异步打开的完整等待时间。

搜索使用实际磁盘遍历和 100 个 64 KiB 文件，每个一个匹配。先完成一次预热扫描，再计时；取消请求由独立线程约在 2 ms 后发出，计时从实际发出开始，五次均观察到取消。该数据不包括 Search UI 的 200 ms debounce 或投递延迟，也不证明所有超长匹配行都能在相同时间取消。

终端测量通过现有 vt100 parser，覆盖 ANSI 和 Unicode 数据流；不包括 PTY I/O、真实 shell、软件渲染和窗口呈现。真实 shell 功能由独立 smoke 验证。

## 修复后性能复测

新基线来自最终源码重新构建后的 release 测试目标，六组各一次预热、五次正式采样，共 31 项指标与 155 个测量值。未通过构建身份核查的中间测量不计入本节或新的结果快照。完整当前数据和相对初始基线的变化保存在 performance.csv、measurements.json 和 performance-comparison.json。

| 指标 | 初始中位数 | 修复后中位数 | 相对变化 |
| --- | --- | --- | --- |
| 1 MiB 每字符输入 | 24.783 µs | 24.001 µs | -3.16% |
| 10 MiB 每字符输入 | 160.189 µs | 168.250 µs | +5.03% |
| 1 MiB 文档粘贴 | 29.033 ms | 26.069 ms | -10.21% |
| 10 MiB 文档粘贴 | 29.334 ms | 27.072 ms | -7.71% |
| 10 MiB 全选缩进 | 289.360 ms | 285.931 ms | -1.19% |
| 64 MiB 文档准备 | 1,589.470 ms | 1,517.557 ms | -4.52% |
| ASCII 长行 100 次 viewport/命中 | 15.800 ms | 15.958 ms | +1.00% |
| Unicode 长行 100 次 viewport/命中 | 13.556 ms | 14.111 ms | +4.10% |
| 多短行 100 次 viewport/命中 | 35.431 ms | 35.230 ms | -0.57% |
| 500 标签 pointer 更新 | 383.356 µs | 400.219 µs | +4.40% |
| 搜索扫描 | 27.298 ms | 25.650 ms | -6.04% |
| 终端解析 | 86.472 ms | 86.965 ms | +0.57% |

本轮耗时指标相对初始基线的中位数上升均低于 10% 调查信号，没有确认持续性能回归。下降值也不应被直接解释为本次检查修复带来的优化。UI session、cached I/O、五个样本和机器条件的限制继续适用。

## 已有问题登记

### 工程与测试问题

| 编号 | 问题 | 证据与处理 |
| --- | --- | --- |
| BASE 01 | 格式检查不通过 | 已修复；只规范 diff_editor.rs 的调用排版，复跑通过 |
| BASE 02 | Clippy 按 CI 参数不通过 | 已修复；11 个原始位置零诊断，无新增 allow |
| BASE 03 | 两项真实 Git 测试继承系统换行配置 | 已修复；仅为临时测试仓库固定 LF，默认和隔离环境均通过 |
| BASE 04 | 部分测试可能内部跳过缺失的运行时 | repository() 与 shell 可用性分支会提前返回或跳过；保留 nocapture 日志，不仅看总通过数 |
| BASE 05 | 工具链可复现条件尚未在项目文件固定 | CI 跟踪 stable；本次实际版本已记录，未执行安装或版本切换 |
| BASE 06 | 历史核心文档含过时入口描述 | 旧文档以 4c89c52 为准，IME 和若干文件位置已变化；本轮新矩阵描述当前行为 |
| BASE 07 | release 的 ConPTY mouse helper 超时 | 补测发现测试二进制继承 windows_subsystem=windows；仅将测试构建排除于该属性之外，正式 release 应用仍保持无控制台 |

BASE 03 的原失败为 git::backend::tests::real_repository_resolves_conflicts_and_continues_a_rebase 和 git_actions::tests::real_repository_switches_with_stashed_changes，原断言分别得到 main 加 CRLF 和 other 加 CRLF。本轮在 cfg(test) 的共享仓库 helper 中明确 LF 策略；未将断言改为忽略换行，也未改变生产后端。配置失败会使测试失败，不静默跳过。

Clippy 原始 11 个位置如下，全部已解决；行号保留为初始定位，不表示当前告警：

| 原始位置 | 诊断 |
| --- | --- |
| src/model/text/mod.rs:21 | ChangeSet 未使用导入 |
| src/app.rs:126 | type_complexity |
| src/file_watcher.rs:82 | type_complexity |
| src/ui/terminal_tab_context_menu.rs:355 | unnecessary_to_owned |
| src/vim/object.rs:96 | chunks_exact_to_as_chunks |
| src/vim/object.rs:101 | chunks_exact_to_as_chunks |
| src/ui/branch_picker.rs:1273 | chunks_exact_to_as_chunks |
| src/ui/diff_editor.rs:1159 | chunks_exact_to_as_chunks |
| src/ui/git_confirm.rs:475 | chunks_exact_to_as_chunks |
| src/ui/git_panel.rs:1230 | cloned_ref_to_slice_refs |
| src/ui/terminal_settings.rs:659 | chunks_exact_to_as_chunks |

两个 type_complexity 通过私有别名表达相同类型；quote 和 RGBA 循环使用完整定长分组，与 chunks_exact 同样忽略不足一组的尾部，顺序不变；测试比较使用借用切片，UI key 使用借用字符串。没有改变编辑、选区、图像通道交换或应用状态更新的语义。

### 依赖候选登记

本轮只登记和收集证据，不执行阶段一的架构强制检查。inventory.json 记录 120 个 AppState 公开字段和 82 处六类词法候选。候选包含 cfg(test) 和其他文本引用，不等于 82 个生产架构违规或完整依赖图。

| 登记 | 当前现象 | 目标移除阶段 |
| --- | --- | --- |
| LEGACY 01 | model 中有 GUI、主题或字体依赖 | R4 |
| LEGACY 02 | Workspace 引用 Git/Search 特定类型与后台释放设施 | R2、R4、R6 |
| LEGACY 03 | model 中有磁盘 I/O 引用 | R4、R5 |
| LEGACY 04 | Git store 依赖 DiffDocument | R6 |
| LEGACY 05 | AppState 引用 UI 类型和运行时句柄 | R1 至 R8 按字段归属迁移 |
| LEGACY 06 | Vim Key 引用应用 Keystroke | R3 |

## 复跑入口与统计规则

Windows PowerShell 7 中，从项目根运行：

~~~powershell
.\scripts\baseline\run.ps1 -Offline
~~~

默认新建 target/baseline/时间戳目录，不纳入源码。也可显式指定一个新的输出目录：

~~~powershell
.\scripts\baseline\run.ps1 -Offline -Samples 5 -OutputDirectory 'C:\path\new-baseline'
~~~

需要已验证版本的 Rust、Cargo、Clippy、rustfmt、Git 和 Node。--offline 使用本机已有依赖缓存；缺少缓存时应记录构建失败，而非静默更新 Cargo.lock。现有 Git 测试会创建临时仓库，真实终端测试会创建并停止临时 shell 或探测进程。

脚本顺序执行格式、Clippy、debug/release 完整测试和正式 release 构建，再复测独立 Git 配置与 native smoke。性能使用 release 并设置 --test-threads=1。每组 sample 0 为预热，report.mjs 仅统计测试成功的 sample 1 至 5，生成中位数和范围。拒绝复用已有日志目录，避免混入不同代码版本的样本。

格式、Clippy 或默认测试失败时，脚本继续收集其余结果，最终返回非零，不能把已知失败变成“绿色”。使用 -PerformanceSet Legacy、Additional 或 None 可选择测量范围；使用 -SkipChecks 可仅采集指定性能组。原生终端 smoke 只在 Windows 执行。

单独重新生成统计：

~~~powershell
node scripts/baseline/report.mjs 'C:\path\existing-baseline'
~~~

比较后续重构时固定输入、工具链、机器、缓存条件和测量方式。较稳定编辑指标可用中位数持续退化 10% 作为调查信号，先重复确认再下结论；低耗时取消、目录遍历和外部 I/O 不套用统一百分比。当前没有设置自动性能 fail 阈值。

## 阶段零验收状态

| 任务 | 本轮结果 | 剩余条件 |
| --- | --- | --- |
| R0.1 工具链和依赖记录 | 已记录版本、平台、锁定提交及 hash | 项目/CI 工具链固定仍需独立处理 |
| R0.2 格式 Clippy 和完整测试 | 本轮已收敛：格式、Clippy、默认及隔离完整测试均通过 | Linux/macOS 与远程 CI 未执行 |
| R0.3 行为矩阵与兼容夹具 | 已建立矩阵及四个读取兼容测试 | 原生 IME、SVG 外观和系统焦点需人工验证 |
| R0.4 性能和 UI session 基线 | 已采集 31 项指标、155 个正式测量值 | 原生窗口延迟分位数、完整软件渲染和进程内存峰值尚未测量 |
| R0.5 反向依赖登记 | 已登记六类候选和移除阶段 | 此阶段不建立目标架构或强制零违规 |

本轮未执行 Linux/macOS 测试、远程 GitHub Actions、发行包构建或原生窗口人工操作，也没有将这些计为通过。阶段零本机自动化检查已全部通过，BASE 01、02、03 和补测发现的 BASE 07 已关闭；工具链固定和原生人工验收仍分别记录。按本轮范围停留在阶段零，不开始架构迁移。
