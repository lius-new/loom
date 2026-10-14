# Loom R7.2 迁移验收

2026-10-14，分支 `refactor/r7-terminal`，基础提交 `dfdcdec`。R7.2 的 PTY、shell 发现、目录集成和原生进程探测迁移已完成当前机器自动验收。停在 R7.2，R7.3 尚未开始。

## 实现与行为

建立 `src/infrastructure/terminal`：`session` 持有控制器、parser 锁、generation、reader/waiter；`pty` 管理 ConPTY/portable-pty 启动、写入资源、resize 和 terminate；`profiles` 处理 PATH、COMSPEC、SHELL、login shell、Git Bash 与自定义可执行路径；`integration` 提供 cwd 验证、启动脚本和临时文件生命周期；`program` 保留原生进程查询、SessionConsole 与隐藏 observer。

`notification::FrameNotifier` 接收 app 提供的同步重绘回调。`app/terminal_runtime` 负责 ApplicationHandle 转换和主题 Palette 注入，基础设施不导入 app、UI、theme 或 GUI 句柄。没有增加线程、队列或异步启动调度。8 KiB reader、2000 行 scrollback、50 ms Windows waiter、250 ms app probe 与原错误文案保持原样。

删除旧 `terminal_profiles.rs`、`terminal_directory.rs` 和 `terminal_program.rs`，不保留旧路径别名。`terminal_session.rs` 暂时保留 TerminalTab/TerminalTabs、相同控制器所有权及原有跨层测试；测试通过仅在 cfg(test) 编译的资源检查接口接线。模型和区域快照仍会保活控制器，其分离属于 R7.3，本轮没有实施。

源码对照覆盖模型生产实现、控制器/reader/waiter/Drop、两平台启动/resize/kill、Windows quoting 与环境合并、shell 发现、目录脚本及原生探测。除声明的路径、模块可见性、回调及 Palette 边界调整，格式化后源码保持一致。Unix 目录原生测试补上纯 DirectoryReports 的 cfg(unix) 导入；这不代表已完成 Unix 编译或运行验收。

## 验证结果

| 检查 | 结果 |
| --- | --- |
| 修改前完整 workspace | 688 通过、0 失败、14 忽略 |
| 修改后完整 workspace | 689 通过、0 失败、14 忽略 |
| 应用 / 编辑核心 / 独立域重复验证 | 592 / 68 / 29 通过 |
| 旧应用执行入口映射 | 605 项全部找到结果，无丢失 |
| 源码测试迁移 | 4 项保留原名，ignored 属性无变化 |
| 架构及报告自测 | 38 通过：架构 35、报告 3 |
| 架构门禁 | 217 个 Rust 源文件；基础设施终端无例外 |
| 全目标编译、fmt、严格 CI Clippy | 通过，无新增 lint 豁免 |
| release 构建、隔离启动和正常关闭 | 通过，退出码 0，synthetic 会话保存成功 |
| 四项 opt-in Windows 原生 smoke | 按完整路径单项运行，全部通过 |

689 = 应用 592 + 编辑核心 68 + 独立域重复验证 29。新增的一项测试验证注入重绘回调、终止状态和旧 generation 拒收；另加强原有 Windows 自定义 shell 测试，实际执行失效回退后修复、重新启动配置程序，并检查参数和提示恢复。完整回归也包含真实 shell/cwd 和 ConPTY wheel 测试。

四项 smoke 保持 `terminal_session::tests::` 精确入口：`supported_shells_round_trip_through_real_conpty_sessions`、`launcher_backed_shell_is_idle_then_detects_and_clears_programs`、`exited_shell_launchers_do_not_hide_unknown_applications`、`running_application_is_detected_and_cleared_in_each_real_shell`。`native_mouse_probe` 仍由鼠标测试调用，未全局执行所有 ignored 项。

门禁拒绝基础设施依赖 app/UI/根状态/GUI、父模块路径绕过、源码包含和迁移例外，并拒绝恢复已删除适配路径。遗留例外文件由 39 减到 36；根/editor/workspace 字段登记仍为 40/31/9，根 UI 适配器仍为 16。按既定 R7.6 范围将 layout_map 的原 R8 例外归属更正为 R7，其余未迁移接线继续明确登记。

## 记录与待验边界

实际工具链为 Rust 1.98.0；项目仍固定 1.97.1，该固定版本当前未验证。Cargo.toml、Cargo.lock、工具链配置、build.rs、observer 源码和原生 smoke 的脚本筛选入口保持原样。

隔离启动只操作本次创建的进程，APPDATA/LOCALAPPDATA 指向测试夹具。记录包括退出码、PID、二进制及源文件 hash；原始日志、冻结修改前源码/测试二进制和最终验证二进制归档到 `target/baseline/2026-10-14-r7-2`，结构化记录位于 `docs/architecture/results/2026-10-14-r7-2`。

原生人工视觉/IME/系统焦点、Linux/macOS、远端 CI 和安装打包验收仍待执行。沿用 R7.1 冻结的同机 before 性能基线，本轮没有完整 after A/B，不宣称性能提升。R7.3 未推进。

本轮 release SHA-256：`FE4BEFC0E4178E61AE301F503289B59C31303888258AC0219B9D590A490A6D9A`。隔离启动 PID：27396；退出码 0，无强制清理。嵌入 observer 构建产物与提取文件 SHA-256 一致。
