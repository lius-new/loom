# R8.1 设置纯规则与平台适配验收

2026-10-14，起点 0caaf0701f35a0b437523f0b06fa0b8550e8e9b9，工作目录 C:\Users\yys\loom，分支 refactor/r8-chrome。R8.1 本机自动验收完成，继续 R8.2；R8 整体仍在进行。

settings 的持久化类型、parse/overlay/patch/template、字体偏好/范围规则、稳定主题 ID 与 JSONC 迁入 features/settings。用户/项目配置文件和 config watcher 迁入 infrastructure，配置路径直接来自 session_filesystem。字体解析/度量、主题调色板和图标注册归 ui；嵌入图标路径同步为新位置。纯主题 ID 与 UI 调色板的顺序/标识通过既有测试新增断言核对。跨工作区配置/会话 JSON 测试归 app，其他纯规则测试保持原断言迁移。

workspace_commands 提前建立 app 接缝，以消费项目纯规则；文件原生操作进一步拆分和旧 workspace_actions 别名/旧 UI 调用归 R8.3–R8.5。此检查点没有声称 root 状态、所有 UI 或 runtime 桥接已收尾。

完整 workspace **711 通过、0 失败、15 ignored**（应用 614、编辑核心 68、域重复验证 29）。629 个原应用入口逐项映射且 ok/ignored 结果一致；51 项源码测试移动，声明 698→698，无新增行为计数、无原覆盖丢失。51 Node 自测（架构 48、报告 3）、门禁、all-targets、fmt、严格 CI Clippy、release 应用构建及隔离启动通过。245 Rust 文件、23 遗留例外；根/editor/workspace 30/30/9 和 12 个根 UI 适配器仍待后续迁移。

27 项函数源码对照通过，保留设置 JSONC 注释、嵌套覆盖、坏字段跳过、默认值删除、字体参数及文件锁/自己的写入过滤。首次资源路径和跨层测试接线错误已修复；首次 Clippy 包装调用丢失 -- 分隔符，显式参数数组重跑通过，原日志保留。release SHA-256 8F76ECE99973A2EA99207895F9FFE43F37297AD2DE3138A8535567B8F0808471；隔离启动 PID 6392 正常退出 0，无强制清理，未修改用户配置或关闭用户进程。

原始记录 target/baseline/2026-10-14-r8-1；R8 起点源码和 before debug/release 二进制在 target/baseline/2026-10-14-r8-base。Cargo.toml/Cargo.lock/rust-toolchain.toml 未变。实际 Rust 1.98.0；固定 1.97.1、最终 R8 性能/隔离/release 全量复验、人工/跨平台/远端 CI/真实安装仍待执行。
