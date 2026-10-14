# R7.4 状态归属与应用命令验收

2026-10-14，分支 `refactor/r7-terminal`，基于 R7.3 检查点 `e71a163ddd5f9d0b2e64a207f44860e4f576c0dd`。本轮完成 R7.4 当前机器自动验收，R7.5 未开始。

## 实现

- 13 个根字段迁出：布局与 sash 归 ApplicationLayoutState；默认终端区域的显示、高度和 resize 归 TerminalChrome；选择、焦点、菜单、滚动、split drag 与 profile 草稿归 TerminalUiState；默认 shell fallback 归纯 TerminalAvailability。editor 的终端双击状态与原有 500 ms/5 px/三击规则一起迁入终端 UI 状态。
- 根状态只组合 layout、terminal_chrome、terminal；六个终端配置字段保留给 R8，设置和会话 JSON 不变。22 项新归属字段登记在 policy；纯会话模型/元数据的 10 项私有登记保持。
- app/terminal 接管新建、选择、split、单个/批量关闭、用途、toggle、输入、剪贴板、鼠标选择与 wheel。命令带 SessionId 或菜单目标，执行前重查存活会话。新建和 split 从目标读取当前 shell/cwd；shell 菜单保留渲染时目标，活动会话变化不会改写已选目标。
- features/terminal/actions 保存纯 close_targets 规则，Close Others/Right 保留目标 split 组。关闭按模型更新、布局修剪、资源释放顺序执行；状态锁不嵌套，完成同一 UI 回合后检查模型与布局归属一致。两个容器之间仍是顺序协调。
- app/terminal_profiles 接管编辑、校验、ID 来源、保存/删除/默认选择及 shell 刷新。配置卡片使用 Frame、Command、StateAccess 和 InputBinding::new_access/render_access；项目 shell 提示由 app 计算。关闭保留草稿，使迟到 blur 可以更新输入状态。
- context_stack 改读新所有者，statusbar/home 改调用 app 命令，settings 传窄绑定；其余 R8 依赖保留。shell 中恢复、启动、轮询、保存、清理 effect 的生产实现仅改字段路径。

## 验收结果

| 检查 | 结果 |
| --- | --- |
| 完整 workspace | **702 通过、0 失败、14 ignored**：应用 605、编辑核心 68、独立域重复验证 29 |
| 旧测试映射 | 613 个旧应用入口无缺失；4 个声明保留原名迁移（profile 3、纯菜单规则 1）；ignored 未变 |
| 新行为测试 | 6 项实际执行通过 |
| 架构门禁 | 231 Rust 源文件；根/editor/workspace 字段 30/30/9；34 个 legacy exception；15 个根 UI 适配器 |
| Node 自测 | 46 通过（架构 43、报告 3） |
| all-targets / fmt / 严格 Clippy | 通过，沿用仓库 CI 的五项既有 Clippy allow 参数 |
| release / 原生 smoke / 隔离启动 | 全部通过，四项 smoke 各实际运行 1 项；隔离启动正常退出，无强制清理 |
| 源码对照 | 21 项通过，涵盖纯模型/规则、PTY、shell 发现、目录集成、原生探测、布局域、profile/剪贴板规则、双击及未迁移的 effect |

新行为测试覆盖：非活动菜单目标的实时 cwd、shell、split 分组和 Close Others；关闭目标后所有命令不误作用于存活会话；最后会话的布局修剪、Stopped 与隐藏；Copy/CopyOrInterrupt/右键 CopyOrPaste/Paste 失败保留选区且无写入；成功复制、Ctrl-C、带 bracketed paste 的 CR 归一化及 replay 字节正确且只到目标会话；detached 会话选择/关闭保持运行时与布局归属；profile 字面参数不展开、项目覆盖和迟到 blur 保留。

原有 UiSession 测试继续覆盖 split 输入、菜单 modal 阻断/非活动目标/关闭焦点、不可用 profile、表单保存和 unmount、默认区域 hide 与 detached focus、布局拖动和移动 PTY 身份。测试字节观测为 cfg(test) 的运行时写入请求记录，原生 ConPTY 交付由已有真实 shell 测试另行验证。

最终 release SHA-256：`73855F509230ECDED3868A3C785DFFEC3B9CC432EA5323E16BA544EFD4C8C474`。隔离启动 PID 19784，退出码 0，独立 APPDATA/LOCALAPPDATA 与临时工作区，未操作用户已运行的 Loom。

## 记录与范围

仓库记录：`docs/architecture/results/2026-10-14-r7-4/{report.md,validation.json,test-migration.json,source-audit.json,source-manifest.json}`。原始日志、冻结源码和最终 debug/release 二进制在 `target/baseline/2026-10-14-r7-4`。

迁移前完整测试 696 通过。首轮迁移后 701 通过，补齐第六项 profile 验收后最终 702 通过；这些是不同版本的验证，不能累加。声明数 682 → 688，与 6 项新增行为一致。编译和门禁接线问题均已修正；没有失败的完整 workspace/native 测试。首次门禁修正被自动审批拒绝，随后改为“登记所有者的声明/文件缺失必须报错”的严格检查并补负向测试，修正版获准执行。

实际 Rust/Cargo 1.98.0。固定 1.97.1、原生人工视觉/IME/系统焦点、macOS/Linux、远端 CI、安装打包及完整 R7 性能 A/B 仍待验。Cargo.toml、Cargo.lock、rust-toolchain.toml 未修改。本轮不迁移 R7.5 effect/通知，也不开始 R7.6 全部终端/布局窄视图。
