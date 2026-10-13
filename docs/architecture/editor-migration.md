# 阶段 3：编辑核心、应用事务与文本 UI

2026-10-13。R3.1 提取独立核心后，本轮迁移编辑事务、交互状态和文本 UI 接口。
本机验收与性能结果见 [本轮报告](results/2026-10-13-r3-transactions/report.md)。
本轮前置工作、独立核心提取和编辑事务/UI 迁移纳入同一 R3 检查点提交。

```text
crates/editor-core/           文本、选区、历史、语义编辑命令、Vim、身份类型
src/app/
  editor.rs                  普通编辑、剪贴板、IME、指针选择、滚动及撤销事务
  editor_vim.rs              Vim Host、请求处理、普通命令与输入回放协调
  editor_keys.rs             应用快捷键 → 核心 Vim Key
  editor_view.rs             当前 pane 展示投影、事件处理、菜单和 Git 覆盖层接线
  state/editor.rs            EditorState、DragSelection、ImeSession 和点击计数
src/ui/
  editor_binding.rs          只读窗格数据、输入意图与回调能力
  editor/
    view.rs                  文本绘制、命中几何、IME 候选窗口坐标和滚动条
    caret.rs                 光标合成动画
    gutter.rs                行号和修改标记
    syntax.rs                语法着色
    menu.rs                  编辑菜单展示与命令意图
    keys.rs                  单行输入框的平台按键 → 核心编辑命令
src/domain/
  identity.rs                核心 FileId/PaneId 的兼容导出
  language.rs                语言身份与路径识别
  line_change.rs             修改标记的纯数据枚举
```

原 src/editor 的普通编辑、Vim 编排、交互状态、视图和菜单入口已迁走，应用调用直接指向
新的归属。model::text、文档/窗格身份和 crate::vim 继续导出相同的核心类型。
阶段零跨系统基线测试移入 app；旧 editor 模块仅保留 Git hunk peek，其加载/操作归入 R6。

## 原子事务与输入目标

Workspace 仍在根 State 的一次更新中修改共享文档及所有关联窗格选区。EditorState 是该
状态中的嵌套值，未创建独立可变文档副本或独立 reactive store。剪贴板读取/写入仍由应用
执行，失败的 Cut 不改动文本或提升 preview；普通命令和 Vim visual 剪贴板共用原有入口。

文本视图只收到当前 pane 的核心文本快照、选区、滚动、光标形状、可见匹配、高亮、IME 和
修改标记。它不持有 AppState、Workspace 或完整 Vim 对象；菜单收到可用性、快捷键标签
和命令回调。lgui State::get 仍先克隆完整根值，再由应用投影，因此本轮收窄能力边界，
不能据此宣称根克隆成本已消除。

指针事件带渲染时的 FileId；目标窗格已切换文件时拒收。IME 会话保留开始时的
PaneId/FileId；晚到提交写入原视图，原视图关闭则丢弃。每个 pane 的候选窗口坐标从自己的
选区和滚动计算；Blur 清理瞬态状态并断开撤销分组。已有真实 UiSession 事件测试继续覆盖
快捷键、拖选、滚动条、Vim、IME 和系统焦点适配。

Git 修改标记投影仅复制当前可见行；overview 保持全文件相邻修改段，继续使用原 10,000
项阈值。hunk peek 的旧实现由 app 提供覆盖层回调，文本视图无 Git store 或加载器访问能力。
Diff UI 继续作为登记的旧适配器；Git 功能迁移属于 R6，其根 UI 接口当前登记至 R8。
它已通过同一窄 binding 使用共享滚动条。

## 所有权门禁

AppState 登记字段从 118 减为 98，其中包含 editor 组合字段和旧运行时弱引用。
20 个字段进入 EditorState，加上原交互状态的 10 个字段，共 30 项逐字段登记。
这是状态归属整理，不是删除业务状态。legacy exception 文件从 76 减为 64。

门禁拒绝文本 UI 重新依赖根状态、Workspace、Vim 状态机、旧 model/editor/Git 模块，拒绝
为文本 UI 添加例外或通过 source include 绕过边界。EditorState 也检查字段登记、过期登记
和运行时服务嵌入。单行输入框直接复用独立核心和 UI 按键转换，不再带入旧 editor 路径。

下一阶段 R4 整理 Workspace/Document/Pane 域与文件、会话适配；继续保留共享文档的一次
原子更新。原生 IME/视觉、macOS/Linux、远端 CI 和真实安装/卸载仍按原计划独立验收。
