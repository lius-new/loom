# R3.1 独立编辑核心提取

2026-10-13。R3.1 已完成并在当前 Windows 机器验收；阶段 3 的应用编辑事务与 UI
接口迁移尚未完成。本轮保留此前的前置工作，所有修改尚未提交。

## 实现

- 新增 workspace 成员 loom-editor-core，拥有 identity、text、normal、commands 和 Vim。
  直接依赖仅 regex、unicode-segmentation 和 unicode-width。
- FileId/PaneId 定义不再带入文件元数据、主题或 GUI 几何依赖。应用 domain 提供共享导出。
- 迁移既有文本、索引、精确变更、历史、普通编辑及 Vim 实现；不改写文本存储算法。
- Keystroke→Vim Key 转换移到 app/editor_keys；KeyboardEvent→Command 保留在应用适配器。
  核心只接收语义命令与 Key；Host/Clipboard 仍由应用提供，保存/退出/滚动为请求。
- 保留 model::text、editor::normal、旧身份路径及 crate::vim 的兼容导出，引用同一份类型
  与实现。Workspace 仍在同一次更新中处理共享文档、历史和各 pane 的选区。
- diagnostics 仅为应用性能测试提供历史字节统计，通过 dev-dependency 开启；release
  默认不启用。清理两处新 crate 中暴露的既有未使用私有成员，无新增 lint 豁免。

使用 `cargo test --locked --offline -p loom-editor-core` 可独立运行核心。
workspace 默认成员包含应用与核心，CI/baseline 也显式使用 --workspace；新增不安装 GUI
构建依赖的独立 core CI job。Cargo.lock 只新增本地 crate 及依赖关系，注册表依赖版本未变。

架构门禁扩展到核心源码和外部宿主测试，拒绝 GUI、应用根状态、直接 I/O、源文件包含逃逸
及迁移例外。逐项删除失效登记，旧例外文件从 89 减为 76；根字段仍为 118 项登记，
本轮没有宣称完成根状态拆分。baseline inventory 包含核心；统计器汇总多个测试套件，
不会让末尾的空 doctest 覆盖测试数量或隐藏失败。

## 最终验收

| 检查 | 结果 |
| --- | --- |
| 独立核心全新构建目录 | 65 unit + 3 external host contract 测试通过 |
| 独立构建环境 | Skia/LLVM 路径故意无效仍通过；依赖树无 GUI/PTY/平台运行时 |
| 默认、独立 Git 配置、release 完整 workspace 测试 | 均 585 通过、0 失败、12 忽略 |
| 原测试保留核对 | 594 条原测试登记经命名空间映射均保留；新增 3 项宿主接口测试 |
| 格式、Clippy 全 workspace/all-targets | 通过，沿用原 CI 参数 |
| 架构门禁/自测 | 154 源文件，12 项负例自测通过 |
| workspace 测试统计器自测 | 3 项通过 |
| 四项真实 ConPTY smoke | 最终均通过 |
| 正式 release 构建及隔离启动 | 通过，存活 4 秒、正常关闭、保存 synthetic 会话 |
| 最终 baseline 命令 | 49/49 exit=0；31 指标、155 样本、0 解析 notices |
| 最终源码 | 全部与采样 inventory hash 一致 |

核心测试覆盖 Unicode/CRLF、undo、peer selection、Vim；新增外部宿主测试验证公共命令、
非法编辑的原子性和保存/退出请求。应用层原有晚到 IME、多 pane、导航、持久化和焦点事件
测试继续运行。原生手工 IME/视觉与跨平台、远端 CI 验收未执行。

## 原生测试与性能调查

首次 launcher smoke 失败，复测及前置版本二进制对照也出现同一失败。诊断发现约 200 ms
的 Bash 启动候选被测试直接当作应用，实际 app/shell 使用 SustainedProgram，持续至少
1 秒才显示标签。只修正 cfg(test) 断言，使用同一生产过滤器；仍验证长期启动标签不能出现，
用户程序能够被检测并在退出时清除。终端生产逻辑未改。修正后连续 5 次及最终完整验收通过。
原始失败、复测与前置版本对照均保留在 target/baseline/2026-10-13-r3-1。

抽取后的首次交错对照发现 10 MiB undo 约慢 25%，因此给语义命令 apply 与 EditorMut::undo
保留跨 crate 内联机会。算法不变。随后前置/当前 release 测试二进制各一次预热和五次样本，
交替执行顺序：十项普通编辑指标变化为 -6.24% 至 +3.21%，10 MiB undo 为 1.0109 ms 对
0.9478 ms。二进制身份、冻结的前次对照和原始日志保留在对应输出目录。

最终常规全套采样相对前置历史中位数仍有信号：1/10 MiB undo 分别 +18.80%/+24.22%，
搜索取消约 +42.97%。交错对照未确认持续的编辑回归；短延迟取消数据不按统一百分比判定。
不声称全部 31 个历史比值低于 10%，也不将下降值归因于本次提取。保留完整数据供后续同机
对照；这些测试不代表原生帧延迟或进程内存峰值。

数据见 [validation.json](validation.json)、[performance.csv](performance.csv)，原始记录在
[最终本机输出](../../../../target/baseline/2026-10-13-r3-1-accepted/measurements.md)。
核心接口与运行说明见 [editor-core](../../../../crates/editor-core/README.md)。

## 后续范围

阶段 3 下一步收窄 app 的编辑命令/事务与 UI 接口，迁移 EditorInteraction 及关联根字段。
保持文档与多视图原子更新、IME 目标校验和旧会话格式；兼容导出按调用迁移逐步清理。
阶段 4–8 尚未开始；真实安装/卸载、原生人工与跨平台验收继续独立登记。
