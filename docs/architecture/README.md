# 阶段 1：接线、状态归属和运行时边界

> 最新状态见 [整体进度](progress.md) 和 [编辑器迁移说明](editor-migration.md)。下文保留阶段 1 的历史截止记录。

阶段 1 建立可逐片迁移的边界。Workspace 仍在原有一次 State 更新中完成跨文档、跨 pane 事务，尚未拆开工作区，也未进入阶段 2 的搜索模块迁移。

## 已落地的目录与职责

```text
src/
  main.rs                       # 保持 Loom 二进制入口和 Windows subsystem 策略
  app/
    mod.rs                      # 私有组合根；对 crate 暴露启动和登记桥接
    bootstrap.rs                # 平台窗口、图标、资源和 lgui 启动
    shell.rs                    # 原根组件的布局、effect 接线；业务事务未拆
    state.rs                    # 旧 AppState；逐字段归属已登记，后续逐片迁移
    runtime.rs                  # 文件加载器、Git dispatcher/poll、搜索取消资源
    presentation.rs             # 窄展示数据和语义命令回调适配
    test_support.rs             # 仅测试：可选择交付顺序的队列和手动时钟
  domain/
    notification.rs             # Toast 数据和时长策略，无 UI 或 I/O 依赖
  features/
    keymap/                     # 原 input 目录；仅接线位置变化
      bindings.rs               # 原 keymap.rs，保留 keymap 兼容导出
      action.rs/context.rs/...  # 原动作、上下文、分发器和 JSONC keymap 逻辑
  infrastructure/
    ui_dispatch.rs              # 原 background_ui：后台结果→UI 通知及回收
  ui/
    toast.rs                    # 接收 Option<Toast> 和 Fn(id)，不读 AppState
    ...                         # 旧根状态接口逐个登记，后续收窄
  model/ editor/ git/ vim/...    # 后续阶段的既有实现；没有批量改名填充空目录
scripts/architecture/
  check.mjs                     # 依赖方向、旧例外、状态字段与桥接门禁
  check.test.mjs                # 门禁负例；新反向依赖必须失败
docs/architecture/
  phase1-policy.json            # 逐字段 owner/remove_in、旧依赖、桥接清单
```

原 `crate::state`、`crate::input`、`crate::background_ui` 是 main 中的 crate 内兼容别名，分别接入 app/state、features/keymap、infrastructure/ui_dispatch。`crate::svg_icons` 仅保留给已有测试。别名不代表目标架构的新公共 API；迁移文件应引用实际的新边界。

## 依赖方向

| 层 | 新代码允许依赖 | 暂存内容 |
| --- | --- | --- |
| app | domain、features、ui、infrastructure 和登记的旧模块 | shell 中仍有生命周期接线与旧命令编排 |
| domain | domain 和纯标准库类型 | 仅通知类型已迁入，model 尚非完全纯域层 |
| features | domain、同层模块；input 是本功能兼容别名 | keymap 保留旧 UI 上下文接线例外 |
| ui | domain、features、ui、theme/fonts/file_icons 和 lgui | 原 State<AppState> 适配器需逐片移除 |
| infrastructure | domain、同层模块、框架通知和后台线程适配 | ui_dispatch 的测试可使用登记测试工具 |

应用接线方向为 `main → app → {ui, features, infrastructure} → domain`。跨功能协作最终通过 app 编排和明确请求/结果，不能由新功能直接持有另一功能的可变内部状态。旧 model/editor/git 等依赖按文件登记；已有文件的例外不能授权新增文件。

## 状态与资源所有权

AppState 原有 120 个公开字段减少到 117 个：移除 file_loader、git_poll_control、git_application；新增一个私有 Weak<AppRuntime> 作为迁移桥接。policy 对现有 118 个字段逐个登记目标 owner 和移除桥接/迁移阶段，不以字段前缀代替逐字段清单。

AppRuntime 的服务槽与轮询控制均为私有。根组件通过独立的 Arc 资源 hook 持有运行时；业务状态克隆只复制弱引用，既不克隆服务槽，也不延长服务寿命。file loader worker 持有旧 State，而旧 State 只弱引用运行时，从而不会构成运行时与 State 的强引用环。effect 负责原有服务安装/清理，运行时 Drop 作为文件加载关闭与搜索取消的兜底。Git poll worker 的 join/drop 仍由原 use_effect 管理，资源拆分没有调整扫描时机。

SearchState 的取消令牌也移到 AppRuntime；查询、结果、代次检查、debounce、匹配、通知合并和导航没有迁移。effect 保留自身的停止令牌，旧 effect 清理只能取消自己的任务，运行时保存当前取消令牌供 Stop/Clear 调用。此项属于资源所有权整理，阶段 2 的搜索命令/端口/服务设计尚未开始。

TerminalTabs 的 PTY 控制器仍处于独立旧 reactive 容器，未混入 AppState；终端运行时、业务模型和展示进一步分离在 R7。SearchState/Settings/InputState 等仍有旧 UI 数据依赖，已登记后续阶段。Weak<AppRuntime> 仍是一项可调用能力桥接，不能声称业务状态已成为纯 DTO，也不能声称所有旧字段已私有化。桥接最终在各功能迁移后移除。

状态目标组合：workspace → domain/workspace（R3/R4）；editor 与 pane 交互 → app/state/editor_interaction（R3）；search → features/search（R2）；explorer → features/explorer（R5）；git → features/git（R6）；terminal → features/terminal（R7）；settings/keymap → 相应功能（R8）；布局和 chrome → app/state。完整字段明细在 phase1-policy.json。

持久化仍使用现有 Settings、WorkspaceSession 等显式 DTO，不序列化 AppState 或 AppRuntime。夹具中的设置、会话和快捷键格式与阶段 0 相同，Cargo.toml、Cargo.lock 和二进制名保持不变。

## 窄接口示例

Toast UI 接收展示快照 `Option<Toast>` 与语义命令 `dismiss(id)`。读取和 State 更新留在 app/presentation；组件不持有根状态，不知道文件加载、Git 或 Workspace。回调仍以 toast id 条件更新，旧卡片点击/旧计时回调不会清除新提示。

lgui 当前 State::get 会克隆整个值。因此本阶段收窄的是组件能力与接口，尚未减少适配器读取成本；不能据此声称 root clone 开销已解决。后续功能拆分才能缩小实际可克隆状态。

## 可执行门禁

```powershell
node --test scripts/architecture/check.test.mjs
node scripts/architecture/check.mjs
cargo fmt --all -- --check
```

需要 Node 20 或更新版本，无新增 npm/Rust 依赖。CI quality job 与 baseline/run.ps1 都先跑门禁和负例测试。门禁对 src 下 Rust 源码扫描，包括 cfg(test)，排除注释、普通/raw 字符串与字符字面量，并识别 crate 根路径、分组 use、外部 import 和受约束标准库服务。

检查拒绝：未登记依赖；新 UI AppState 适配器；未登记 runtime() 桥接；未登记 AppState 字段；把 FileLoader/ApplicationHandle/PollControl 直接嵌回根状态；已移除但未清理的依赖例外/桥接/字段。domain 禁止通过 super/路径属性逃出层边界。policy 不提供自动更新例外命令，新增例外需要明确理由、目标阶段和代码审查，不能为通过检查批量刷新旧债务。

这是保守的词法门禁，追踪到 crate 根模块；它不解析 Rust 类型别名、宏展开、全部路径解析或模块循环。Rust 编译/Clippy 和后续 crate 拆分的编译边界仍有必要。不要把此脚本当成全面的 AST 架构证明。

## 测试工具和验收

UiTasks 为 posted UI 回调分配递增 id；测试可以 run(id) 选择完成顺序，重复交付返回 false，run_all 只交付当前批次且在锁外调用。ManualClock 提供可推进的 elapsed Duration；已用于文件加载 299ms 指示器断言和旧通知计时断言。生产计时器、后台线程与调度机制保持现状，测试工具不编入正式应用。

阶段 1 的新增测试覆盖：状态快照不保活运行时；独立 dispatcher 与幂等清理；只销毁运行时才取消搜索；任意 UI 交付顺序与旧计时结果拒收；回调可重入投递；Toast 实际点击回调和空展示。已有文件加载、Git 过期结果、Workspace 多 pane/undo、搜索、终端、IME 事件和持久化测试继续参与完整回归。

本地自动验收证据与性能比较见阶段 1 执行报告。Windows 原生 IME/候选窗口、视觉与焦点手工验收、macOS/Linux、远端 CI 和安装包验收仍为阶段 0 保留的待验项，不被本地测试替代。
