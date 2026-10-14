# Loom R7.1 迁移验收

2026 年 10 月 14 日，分支 `refactor/r7-terminal`，基础提交 `b414223`。R7.1 的纯规则提取与应用布局域迁移已完成本机自动验收，停在此检查点，R7.2 尚未开始。

## 已完成的变更

- 建立 `src/features/terminal`：ShellKind、SessionId、屏幕/单元格 DTO、ANSI 颜色及 run、选区、粘贴、键盘字节、滚轮协议、OSC 9;9、shell 参数和候选回退、稳定程序标签及给定进程信息的纯选择规则。
- 屏幕颜色由调用方传入 Palette，纯规则不再读取 theme。UI 的 KeyboardEvent/KeyModifiers 转换位于 `src/ui/terminal/keys.rs`，协议编码接收无 GUI 的键和修饰键。
- 将 `src/model/application_layout.rs` 迁入 `src/domain/application_layout.rs`，使用现有 Rect、Axis、Direction；app/UI 在边界转换坐标。布局规则及其测试的词法 token 在坐标类型和测试路径替换后完全一致，浮点计算、计划提交校验和尺寸规则保持原样。
- 删除旧 `terminal_mouse.rs` 与 `model/application_layout.rs`，不保留旧路径别名。新增门禁拒绝纯终端规则中的 GUI、根状态、运行时、I/O、环境服务、源码包含与例外，并拒绝恢复已删除路径或从旧模块重新导出终端类型。
- 清理实际失效的依赖登记，遗留例外文件从 41 减到 39；根状态 40 项、editor 31 项、workspace 9 项及 16 个根 UI 适配器保持原归属。

PTY、控制器、运行时所有权、shell 发现/启动、临时集成脚本、OS 进程探测、effect、终端会话列表和旧 UI 根适配继续留在当前实现中，属于后续 R7.2–R7.6。本轮没有建立 infrastructure/terminal，没有实施模型/控制器分离，也没有改会话 JSON、依赖、工具链配置或二进制名。

## 回归结果

| 检查 | 结果 |
| --- | --- |
| 修改前完整 workspace | 673 通过、0 失败、14 忽略 |
| 修改后完整 workspace | 688 通过、0 失败、14 忽略 |
| 修改后应用 | 591 通过、14 忽略 |
| 编辑核心 unit/integration | 68 通过 |
| 独立域重复验证 | 29 通过，其中新增 13 项为同一份布局规则的重复验证 |
| 架构及报告自测 | 36 通过，其中架构 33、报告 3 |
| 架构门禁 | 通过，212 个 Rust 源文件，纯终端规则与布局域无例外 |
| 全目标编译、fmt、严格 CI Clippy | 通过，Clippy 零警告，无新增豁免 |
| release 应用构建 | 通过 |
| Windows 原生终端 smoke | 四项逐项通过，均实际执行 1 项测试 |
| 隔离 release 启动 | 正常关闭、退出码 0，synthetic 工作区保存成功，无强制清理 |

688 = 应用 591 + 编辑核心 68 + 独立域重复验证 29。相对修改前增加 15 个执行通过项，其中只有 2 项是新增行为覆盖，另 13 项是复用生产布局源码的独立验证，不能全部算作新增行为测试。

34 项旧源码测试迁移后保留原名；整体源码测试声明从 672 增至 674，无缺失、无 ignored 属性变化。修改前冻结应用二进制的 603 项测试（含 14 ignored）全部完成名称映射，并在修改后回归日志中找到对应结果。源码声明包含平台 cfg，不等于实际执行数。

新增行为测试验证调用方调色板控制 ANSI 与反色默认值，以及 Ctrl 字符、工作区保留快捷键、普通文字与粘贴路径的协议分工。既有 GUI 修饰导航测试仍经实际事件适配器执行。

四项 opt-in 原生测试为 shell round-trip、launcher 检测、Git Bash launcher 替换及运行程序名称检测。ConPTY wheel 的非 ignored 回归也参加了完整 workspace。未对所有 ignored 测试执行全局运行，native_mouse_probe 仍由所属测试作为辅助入口调用。

## 当前机器性能基线

从冻结的修改前 `b414223` 源码重建 release 测试二进制，在当前机器采集六组原有基线：每组预热 1 次、正式 5 次，31 项指标、155 条正式测量记录，36 次执行全部通过，解析 notices 为 0。采样时当前 checkout 已完成 R7.1，但测量使用的是冻结的修改前二进制。

该记录作为后续 R7 的同机 before 基线，本轮没有做完整 after A/B，也不宣称性能提升。终端的 `terminal_screen_snapshot_100` 仍测量 screen.contents()，不代表完整生产快照或原生帧延迟。R7 专项生产快照/帧构建对照留给后续收尾。

## 工具链与待验边界

当前机器实际使用 Rust 1.98.0；项目 `rust-toolchain.toml` 仍固定 1.97.1，当前没有可用的 1.97.1 工具链或 rustup 入口。本轮通过结果不能替代固定 1.97.1 的 CI 验证。Cargo.toml、Cargo.lock 和工具链配置内容保持不变。

原生人工的颜色/字体/IME/系统焦点、macOS/Linux、远端 CI、打包及真实安装验收尚未执行。隔离启动只操作本次创建的 PID 20828，APPDATA/LOCALAPPDATA 使用测试目录，保留用户已有 Loom 进程与真实会话。

release 应用 SHA-256：`DF7DCF1FF147E8E59F3715CC00A9E32B870C174026524B1139C48DBE2083160E`。

项目中的结构说明为 `docs/architecture/terminal-migration.md`；验证记录与测试映射位于 `docs/architecture/results/2026-10-14-r7-1`。原始日志、冻结二进制和性能样本归档到 `target/baseline/2026-10-14-r7-1`。下一步仍是 R7.2，本次未执行。
