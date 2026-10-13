# 阶段 3 前置工作与进入条件

2026-10-13，当前机器 D:\Documents\cprojects\loom。基础提交为
`8cb46fbe76588e5b964c60c9bc222ff80c261ae4`。本轮完成本机可复现构建、
自动回归与性能基线准备；阶段 3 尚未开始。应用 Rust 源码没有修改。

## 构建恢复和工具链

原默认 Rust 为 1.96.0，历史阶段 2 验收使用另一机器上的 1.98.0。
本机已安装且本轮验证通过的 Rust 1.97.1 现由 rust-toolchain.toml 固定；
CI 与 Windows release workflow 同步指定 1.97.1。Clippy、rustfmt 一并固定。

初始失败发生在 skia-bindings 的构建脚本：Cargo 离线模式仍允许该脚本
自行尝试下载；下载失败后转为需要独立 LLVM 的源码构建，本机没有该安装。
本轮找到现有缓存中与锁定版本、Windows x64 目标和特性匹配的预编译归档，
核对内置 tag/key，并通过上游支持的 file:// URL 成功构建。
归档 SHA-256 为 `c410ec64ed356b11ba877847889b279daf52e6972ada79137493931668c12b79`。
该 digest 来自本地缓存内容核验，未声称独立验证上游签名。

新增构建 helper 自动发现并校验缓存；baseline 脚本使用相同入口并记录缓存
身份。版本、路径或内容不匹配时失败，环境覆盖在退出时恢复。缓存保存在
git-ignored 的 target 中，不复制到源码或修改依赖。说明见
[构建说明](../../../../scripts/build/README.md)。

## 本轮验收

| 检查 | 当前机器结果 |
| --- | --- |
| 架构门禁、负例自测 | 通过；146 个 Rust 文件，118 个登记字段；10 项自测通过 |
| 格式、Clippy 全目标 | 通过，使用既有 CI 参数，无新增 lint 豁免 |
| 默认 debug 完整测试 | 582 通过、0 失败、12 忽略 |
| 独立 Git 配置 debug 完整测试 | 582 通过、0 失败、12 忽略 |
| release 完整测试 | 582 通过、0 失败、12 忽略 |
| 正式 release 构建 | 通过 |
| 四项独立真实 ConPTY smoke | 各 1 项通过 |
| baseline 检查及性能采样命令 | 48/48 exit=0 |
| Skia 缓存与 wrapper 守卫 | 9 项通过；含损坏、版本变化、环境恢复及 Cargo 失败传递 |
| release 隔离启动 | 存活 4 秒、正常关闭 exit=0、保存 synthetic 工作区；无强制清理 |
| Windows portable 打包、内置 Git smoke | 通过 |
| Windows Inno Setup 安装器编译 | 通过；未执行安装/卸载 |
| packaged 应用隔离启动 | 通过，和 release 应用 SHA-256 相同 |

启动脚本仅启动和关闭自己的 PID，APPDATA、LOCALAPPDATA 指向独立测试目录。
第一次脚本断言未处理 Rust 保存路径的 Win32 verbatim 前缀；应用已正常退出，
修正路径校验后在新的目录复跑通过，初次失败记录未覆盖。
隔离启动不等于 IME、外观或系统焦点的人工验收。

收尾还修正了 PowerShell/.NET 将不存在的环境变量恢复为空字符串变量的问题。
Skia override 和独立 Git 配置现在分别保留变量的存在性和值，补测缺失、空值
和既有值的恢复。此项只涉及检查脚本，应用源码与已采样的 release 二进制未变。

当前 146 个源文件的 SHA-256 与历史阶段 2 inventory 全部相同。
Cargo.toml hash 相同；Cargo.lock 原始 hash 的差异来自 CRLF/LF，统一为 LF 后
hash 为 `8E60A3117C5294406747E4964D3E69B2A31AB7E380F6E9ACD40AA0BB86DBCC0C`，
与历史记录相同。没有更新依赖或锁文件。

## 当前机器性能基线

六组 release 测量，各一次预热与五次正式采样；31 项指标、155 条记录，
没有解析 notices。机器和编译器与历史验收不同，不对两台机器的结果计算
回归比例或声称性能优化。阶段 3 应与本轮同机记录比较。

| 指标 | 当前中位数 |
| --- | --- |
| 1 MiB 文档逐字符输入 | 21.532 µs/字符 |
| 10 MiB 文档逐字符输入 | 223.021 µs/字符 |
| 64 MiB 文档准备 | 951.4924 ms |
| 100 文件 / 6.25 MiB cached 搜索 | 21.1080 ms |
| 搜索取消请求到返回 | 0.0747 ms |
| 500 标签 pointer 更新 | 0.23895 ms |
| 约 8 MiB ANSI/Unicode 终端解析 | 58.7211 ms |

完整统计见 [performance.csv](performance.csv)；检查记录、环境、构建产物和
脚本 hash 见 [validation.json](validation.json)。原始日志及逐样本数据位于
[本地输出](../../../../target/baseline/2026-10-13-before-r3/measurements.md)。
CIM 硬件详情查询被当前执行环境拒绝，环境记录保留 OS、处理器标识和逻辑
处理器数量；不把缺失硬件详情填成历史机器的信息。UI session 和 cached I/O
测量不代表原生窗口延迟分位数、完整渲染或进程内存峰值。

## 阶段 3 的接续约束

本机自动前置条件已满足。下一步从 R3.1 开始，按下列顺序细化实施：

1. 明确 FileId/PaneId 等身份类型的无 GUI 归属，解除 Vim 对旧 model 的带入依赖。
2. 提取现有 text、selection、history 与普通编辑行为，保持索引及精确变更算法。
3. 把平台 KeyboardEvent/Keystroke 转换留在适配层，核心接收语义命令和 Vim Key。
4. 保留 Host/Clipboard 请求边界，并建立不依赖 lgui/Skia 的独立编译和测试入口。
5. 再收窄应用编辑事务和 UI 接口；保持共享文档、多 pane 选区变换的一次原子更新。

验收应包含核心独立构建；Unicode/CRLF、undo 分组、跨 pane 选区、Vim、晚到 IME
提交的行为保持；全应用回归、架构门禁与本机性能对照；逐项清理已失效的例外。
先提取边界，文本存储算法或 reactive 状态源调整需另有证据和独立验收。

## 仍未覆盖的验收

- 原生中文 IME、候选窗口坐标、视觉布局及系统焦点手工检查。
- Linux/macOS 的实际构建、测试和 native PTY 验收。
- 远端 GitHub Actions 的实际运行；本轮仅更新其工具链配置。
- 安装器真实安装、升级与卸载路径。生成安装器不等于安装验收。

这些项目继续保留为人工、跨平台和发行验收，未计入本机自动前置完成范围。
