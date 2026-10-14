# R8.2 设置与快捷键状态 端口验收

2026-10-14，起点 924071522f667cc73984bd838d732e727748bb2d，分支 refactor/r8-chrome，工作目录 C:\Users\yys\loom。R8.2 本机自动验收完成，继续 R8.3，R8 整体仍在进行。

九项根偏好值（theme/default_shell/profiles/options/blink/smooth_caret/editor_font/terminal_font/project_settings）归 features/settings 私有 Model；Arc 仅持纯 Values，写时复制保留旧快照，不含 GUI、进程或服务。Git 展示选项与 Vim 编辑选项保持已有所有者，app 组合显式 Settings DTO，序列化格式不变。字体输入草稿归 ui/settings_binding。设置页只读帧/局部投影/语义命令，toggle、字体 commit/cancel/step、滚动、打开配置由 app/settings 执行；原 rescale/trim/blur 顺序保留。

keymap 上下文由 app/keymap_context 投影 ContextFacts，纯规则保留 modal 优先、输入/搜索/终端/editor/Vim/menu 次序。GUI 键事件在 ui/keymap_events 解码，大小写/Shift/AltGr 和多字符判定留在纯 keystroke；文件 I/O 迁入 infrastructure/keymap_filesystem，编排迁入 app/key_actions。序列、代次和版本归私有 feature 模型，filter InputState 归 UI。Keymap 页面仅收不可变 Keymap 值、滚动/位置字符串、filter 投影和命令；原有 keymap 兼容别名已删除。配置自己写入的文件检查下沉基础设施。

完整 workspace **713 通过、0 失败、15 ignored**（应用 616、编辑核心 68、域重复验证 29）；629 个 R8.1 原应用入口逐项映射无缺失，23 个源码测试迁移，698→700 个声明。新增 2 项验证配置快照独立性及纯中文/多字符/未知键事实边界。原 Setting/Font、上下文/Vim、5 项 GUI 键事件断言保留迁移。53 Node 自测（架构 50、报告 3）、门禁、fmt、全 targets 严格 CI Clippy、release 与隔离启动通过。254 Rust 文件、19 根字段、15 遗留例外、10 根 UI 适配器；设置/keymap 无迁移例外，模型存储私有且逐字段登记。

13 项规则/资源文件与 R8.1 源码完全一致。首次接线编译、旧别名、未使用导出和多余借用错误已修复，原日志保留；最终完整回归与最终源码对应。release SHA-256 44F0158EAA43F1514F21916B52E906B50FB2C6BCD90C0CF1D807EE48DEEA98F0，隔离启动 PID 20904 正常退出 0，无强制清理，不修改用户配置。

原始记录 target/baseline/2026-10-14-r8-2。Cargo/lock/toolchain 未改，实际 Rust 1.98.0。后续文件/会话服务、chrome、root runtime 桥接和最终性能/默认隔离 release 全量复验尚未完成；固定 1.97.1、人工/跨平台/远端 CI/安装仍待验，不宣称整个 R8 完成或整体根克隆成本已消除。
