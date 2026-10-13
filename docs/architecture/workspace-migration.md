# 阶段 4：工作区域规则与文件、会话适配

2026-10-13，接续 R3 检查点 4895cc9。Workspace、Document、Pane 的规则已迁入 domain；
文件观察、会话存储、Git 载荷接入和后台回收由外层协调。
本机验收与性能结果见 [R4 报告](results/2026-10-13-r4-workspace/report.md)。
本轮纳入 R4 检查点提交。

```text
src/domain/
  document.rs                FileMeta、DiskState 和 Found/Missing/Failed 磁盘观察
  geometry.rs                不依赖 lgui 的逻辑 Rect
  pane_layout.rs             原有分屏树、邻居、尺寸约束、sash 和布局规则
  workspace.rs               共享文档、pane/tab/view、dirty/undo、加载及冲突规则
  workspace/tests.rs         注入观察的纯规则测试，不访问磁盘
  workspace_session.rs       稳定 editor DTO、树快照、pending tab 和恢复形状
src/app/
  workspace.rs               单一工作区组合容器，供根 State 一次更新
  workspace_git.rs           原 Diff/Git 请求、刷新和 UI 行高适配
  workspace_session.rs       会话 envelope、保存/恢复协调及既有运行时锁
  workspace_restore.rs       原延迟恢复 worker、停止/重试和 UI 安装协调
src/infrastructure/
  workspace_filesystem.rs    文件读取、元数据与 hash 的观察准备
  session_filesystem.rs      JSON、配置路径、sync/rename 原子替换及临时文件清理
src/ui/
  geometry.rs                lgui 与域坐标的转换
  language.rs                语言 badge 的颜色策略
crates/workspace-tests/      仅验证：直接编译同一份生产 domain 源码，无 GUI/app/Git
```

## 事务与状态归属

app::Workspace 组合泛型 domain::Workspace<ReadonlyPayload, SectionKey>。应用以原 DiffDocument
和 DiffRequest 提供只读载荷及节标识；域规则不知道 Git backend、请求执行或 UI 行高。
这里没有复制工作区或建立另一套 reactive store。FileId/PaneId、同文档共享缓冲、历史、
dirty 缓存、每 pane 选区/滚动、preview 与 tab 顺序沿用原实现。

域仅返回待回收的数据；应用的事务入口把它交给既有后台回收器。关闭、preview 替换、移动、
prepared 安装与磁盘更新仍在一次根 State 更新中完成。Deref 转发保留旧读取/编辑 API；
可能产生回收数据的入口由应用显式包装。新增测试覆盖已有路径的 preview 复用与跨 pane
移动，确保事务完成后不残留待回收队列。

Workspace 的 9 个字段逐项登记在 workspace_core_ownership。根状态仍登记 98 项，editor
子状态 30 项；本轮不删除业务字段，也未减少 State::get 的根克隆范围。

## 文件观察与异步安装

metadata/read/hash 的 I/O 在基础设施准备。域接收 DiskState 和 DiskRead；Missing 与 Failed
保留原区分，干净文档可重载，脏文档保留编辑内容并提示冲突。接受磁盘版本会清空历史并
钳制所有关联选区，保持滚动；保留编辑版本、mark_saved 和路径重定向接收明确磁盘状态。

PreparedDiskUpdate 继续校验 FileId、路径、原磁盘状态、saved snapshot 身份和文本版本。
晚到结果不能覆盖保存、重命名、关闭或后续编辑。加载占位文档继续禁止编辑、保存和磁盘
重载；worker 准备文本与索引，UI 只安装准备结果。文件读取限制、线程调度与 300 ms
指示器规则保持原样；原恢复任务的停止和重试行为未重写。

## 会话兼容

EditorLayout/SavedAxis/SavedPaneNode/SavedChild 的 serde 属性、名称和默认值原样迁入域层。
快照仍过滤非文件 tab、保留 preview 和 pending 文件顺序、剔除空 pane 并归一化尺寸。
恢复先创建占位 tab，再安装 worker 结果。整个会话 envelope 留在 app，原终端、目录、窗口
和 source control 字段继续保持，不把它们并入 editor 域。

会话存储继续使用进程锁、create_new 临时文件、sync_all、rename 和失败清理。配置路径及
旧 JSON 夹具未变化。跨平台/真实安装路径验收仍按原计划待执行。

## 可执行边界

```sh
cargo test --locked --offline -p loom-workspace-tests
cargo test --locked --workspace
node --test scripts/architecture/check.test.mjs
node scripts/architecture/check.mjs
```

workspace-tests 不成为应用依赖；它仅用编辑核心、serde 和 serde_json 编译当前 src/domain。
其测试使用 isolated::domain 前缀，完整 workspace 报告单列这部分重复规则验证，不把它算作
新的行为覆盖。门禁同时约束该入口的源路径与依赖清单，并禁止域规则重新依赖 app、GUI、
Git、I/O 或添加迁移例外。

旧 model/document 与 model/pane_layout 是纯域导出；model/workspace 是指向 app 组合
容器的登记兼容路径。workspace_persistence/session_restore 的根别名指向应用模块。
model/diff_document、application_layout、旧 UI 根接口以及 Explorer/文件命令中的旧接线
继续按 R5–R8 登记迁移；本轮不宣称这些模块已全部纯化。下一步 R5 整理 Explorer。
