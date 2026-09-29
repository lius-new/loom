# Loom Git 集成完整计划

## 1. 文档目的

本文档定义 Loom 的完整 Git 集成路线。目标不是简单增加若干 Git 命令，而是在 Loom 中建立一套可长期演进的 Git 运行时、仓库状态引擎和编辑器工作流，使用户只安装 Loom 就能获得成熟、可靠、低感知的版本控制体验。

本文档同时作为后续实现、评审、测试和发布验收的基线。若实现过程中需要调整架构或范围，应同步更新本文档，而不是只在代码中形成隐含约定。

## 2. 产品目标

Loom 的 Git 支持应达到以下体验目标：

- 官方发布包默认包含可用的 Git runtime，普通用户无需额外安装 Git。
- 用户不需要理解 Git runtime、askpass、credential helper 或 porcelain 输出等底层概念。
- 文件树、编辑器 gutter、Git 面板和状态栏始终展示一致的仓库状态。
- Loom 内的操作与外部终端中的 Git 操作可以安全共存，并能及时互相反映。
- 用户能够在不打开终端的情况下完成日常本地、远程和冲突解决工作流。
- Git 操作不得阻塞 UI，不得静默覆盖未保存内容，也不得因为过期扫描结果回滚 UI 状态。
- 多根工作区、嵌套仓库、submodule 和 worktree 必须从数据模型层面得到支持。
- 高级用户可以显式选择系统 Git 或自定义 Git，但这不是普通用户的前置步骤。

### 2.1 首个完整版本的功能范围

首个完整版本应包含：

- Managed Git runtime；
- 仓库发现与多仓库状态管理；
- 文件树 Git decoration；
- Git 面板；
- 文件级和 hunk 级暂存、取消暂存及恢复；
- commit、amend 和 undo commit；
- unified/split diff、word diff 和 gutter 标记；
- 分支、tag 基础能力；
- commit history、文件历史和 blame；
- stash 和 worktree；
- clone、fetch、pull、push；
- HTTPS/SSH askpass 交互；
- merge/rebase/cherry-pick 状态识别与冲突解决；
- submodule、LFS 和 sparse checkout 的状态识别及基础操作；
- 完整的错误分类、日志、取消、恢复和诊断能力。

### 2.2 非目标

以下内容不属于 Git 核心层的首要目标：

- 自行实现 Git 对象数据库、传输协议或服务端；
- 用 Rust 重写完整 Git；
- 将 GitHub/GitLab Pull Request、Issue 等托管服务能力混入通用 Git 层；
- 在尚无第二种版本控制系统需求时提前设计通用 SCM 插件 API；
- 为了覆盖极少数命令而复制整个 Git 终端界面。

GitHub、GitLab 等托管服务集成应作为后续独立的 `HostingProvider` 层存在。

## 3. 已确认的核心决策

### 3.1 默认使用 Loom Managed Git

官方发布包默认携带 Loom 测试过的固定 Git runtime：

- Windows x64：优先使用 Git for Windows 的 MinGit 发行物；
- macOS ARM64：在 Loom 应用包或发行目录中携带 Git 和必要 helpers；
- Linux x64：官方便携发行包携带受控 runtime；发行版原生包可以选择系统依赖，但不能改变上层架构；
- WSL、SSH、Dev Container 等远程环境：在目标环境中执行 Git，不能使用宿主机 runtime 操作远端文件系统。

普通用户默认不会看到 Git runtime 选择界面。高级设置中允许选择：

```text
Loom Managed Git
System Git
Custom Git Path
Remote Environment Git
```

选择逻辑：

1. 管理策略或用户明确指定的自定义 Git；
2. Loom Managed Git；
3. 系统 Git 作为故障回退；
4. 若均不可用，显示带修复建议的诊断页。

### 3.2 Git CLI 作为权威执行后端

Git CLI 负责：

- index、refs 和 worktree 写入；
- commit、merge、rebase、cherry-pick、revert；
- fetch、pull、push 和 Git 网络协议；
- SSH、HTTPS、credential helper；
- hooks、签名、LFS、submodule、sparse checkout；
- Git 配置和兼容性语义。

Loom 负责：

- runtime 选择；
- 仓库发现；
- 状态缓存和增量刷新；
- 操作排队、取消、进度和错误分类；
- diff 展示与内存 buffer 差异；
- askpass 和认证 UI；
- merge editor；
- 用户工作流和安全边界。

### 3.3 禁止 UI 直接执行 Git

UI、编辑器和 workspace 模块不得直接出现 `Command::new("git")`。所有 Git 操作必须经过统一后端。

当前 `src/workspace_actions.rs` 中的 clone 实现需要迁移到 Git 后端，并保留现有异步、不阻塞 UI 的行为。

### 3.4 状态与运行时分离

当前 `AppState` 是可克隆的 UI 根状态。线程、子进程、watcher 和 channel 不应直接进入 `AppState`。

采用以下分层：

```text
Arc<GitService>
  持有 runtime、worker、watcher、任务队列和进程

State<GitStoreSnapshot>
  持有 UI 可读取的不可变仓库快照

AppState
  仅持有 Git 面板选择、展开状态、对话框等 UI 状态
```

任何 Git I/O 都不得发生在 render 函数或 `AppState::update` 闭包中。

## 4. 总体架构

```text
┌───────────────────────────────────────────────────────────┐
│ Git Panel / File Tree / Diff / Gutter / History / Status │
└────────────────────────────┬──────────────────────────────┘
                             │ semantic commands
                             ▼
┌───────────────────────────────────────────────────────────┐
│                     GitStore Snapshot                     │
│ repositories / active repo / file status / operations    │
└────────────────────────────┬──────────────────────────────┘
                             │ events
                             ▼
┌───────────────────────────────────────────────────────────┐
│                       GitService                          │
│ discovery / watchers / repository actors / scheduling    │
└────────────────────────────┬──────────────────────────────┘
                             │ typed requests
                             ▼
┌───────────────────────────────────────────────────────────┐
│                      GitBackend                           │
│ status / diff / stage / commit / branch / remote / log   │
└────────────────────────────┬──────────────────────────────┘
                             │ argv + stdin/stdout/stderr
                             ▼
┌───────────────────────────────────────────────────────────┐
│                     GitRuntime                            │
│ managed / system / custom / remote                       │
└───────────────────────────────────────────────────────────┘
```

## 5. 建议的代码结构

```text
src/git/
├── mod.rs
├── runtime.rs        # runtime 发现、版本、来源、能力检测
├── command.rs        # 进程启动、流式输出、取消、超时、脱敏
├── backend.rs        # GitBackend trait 和 CliGitBackend
├── types.rs          # 公共领域类型
├── parser.rs         # porcelain/log/branch/diff 输出解析
├── discovery.rs      # 多根、嵌套仓库、worktree、submodule 发现
├── repository.rs     # 单仓库 actor 和操作队列
├── store.rs          # 多仓库快照与事件
├── watcher.rs        # 工作树和 .git 文件监听
├── diff.rs           # HEAD/index/disk/buffer 差异
├── operations.rs     # stage/commit/branch/stash/remote 等
├── auth.rs           # askpass、IPC、secret 生命周期
├── history.rs        # log、blame、cat-file batch
└── error.rs          # 错误分类和用户可读信息

src/ui/
├── git_panel.rs
├── diff_view.rs
├── branch_picker.rs
├── commit_editor.rs
├── merge_editor.rs
├── git_auth_dialog.rs
└── git_operation_log.rs
```

如果后续模块规模较小，可以先合并文件，但领域边界应保持一致。

## 6. 核心数据模型

### 6.1 Runtime

```rust
enum GitRuntimeSource {
    Managed,
    System,
    Custom,
    RemoteEnvironment,
}

struct GitRuntime {
    executable: PathBuf,
    root: PathBuf,
    source: GitRuntimeSource,
    version: GitVersion,
    capabilities: GitCapabilities,
}
```

`GitRuntimeManager` 负责定位、校验和选择 runtime；`CliGitBackend` 只接收已解析完成的 `GitRuntime`。

### 6.2 仓库快照

```rust
struct RepositorySnapshot {
    id: RepositoryId,
    worktree_root: PathBuf,
    git_dir: PathBuf,
    common_dir: PathBuf,
    head: HeadState,
    upstream: Option<UpstreamState>,
    ahead: u32,
    behind: u32,
    files: BTreeMap<RepoPath, FileState>,
    operation: Option<OperationState>,
    repository_state: RepositoryState,
    generation: u64,
}
```

快照必须是可克隆、可比较和只读消费的。UI 不得持有后端对象引用。

### 6.3 文件状态

Git 状态必须保留 index 和 worktree 两个维度：

```rust
struct FileState {
    index: ChangeKind,
    worktree: ChangeKind,
    conflict: Option<ConflictKind>,
    original_path: Option<RepoPath>,
}
```

这用于正确表示：

- `M `：已暂存修改；
- ` M`：未暂存修改；
- `MM`：暂存后继续修改；
- `AM`：新增已暂存后继续修改；
- rename/copy；
- untracked/ignored；
- 所有冲突组合。

不能将上述状态压平成单个 `Modified`。

### 6.4 仓库过程状态

```rust
enum RepositoryState {
    Normal,
    MergeInProgress,
    RebaseInProgress,
    CherryPickInProgress,
    RevertInProgress,
    Bisecting,
    UnbornBranch,
    DetachedHead,
}
```

### 6.5 操作状态

```rust
struct OperationState {
    kind: OperationKind,
    started_at: Instant,
    progress: Option<f32>,
    cancellable: bool,
    message: String,
}
```

所有长操作必须以状态形式暴露给 UI，而不是只在结束时返回结果。

## 7. Git 命令和输出规范

### 7.1 进程规范

- 使用参数数组启动进程，不经过 PowerShell、cmd、bash 或 sh。
- 所有路径参数前使用 `--` 结束 option 解析。
- stdin、stdout、stderr 分别管理。
- 支持取消并终止完整进程树。
- 后台命令禁止交互式终端 prompt。
- stdout/stderr 日志必须脱敏。
- 不在普通日志中输出 token、密码、Authorization header 或包含凭据的 URL。

### 7.2 状态命令

初始实现使用：

```text
git status --porcelain=v2 --branch -z --untracked-files=all
```

要求：

- 以原始字节解析 NUL 分隔输出；
- 不先对完整输出执行 UTF-8 转换；
- 支持 rename、特殊路径、中文路径和换行路径；
- 解析 HEAD、upstream、ahead、behind；
- 支持 unborn branch。

### 7.3 后台只读命令

后台命令应尽可能使用：

```text
--no-optional-locks
--no-ext-diff
--no-color
```

同时禁止意外触发外部 diff、外部 pager 或终端认证。

### 7.4 常用写操作

```text
git add -- <path>
git restore --staged -- <path>
git restore -- <path>
git commit --file=-
git switch <branch>
git switch -c <branch>
git fetch <remote>
git pull [--rebase]
git push [--set-upstream]
```

commit message 通过 stdin 传递，不拼接到命令行。

## 8. 仓库发现

### 8.1 多根工作区

每个 workspace folder 独立发现仓库。一个工作区可以同时激活多个 repository。

文件归属规则：

- 文件关联包含它的最近仓库根；
- 若 workspace folder 本身是仓库子目录，向父级发现仓库时必须遵守 workspace trust；
- submodule 可表现为独立仓库，同时保留与父仓库的关系；
- linked worktree 的 `.git` 文件和 common dir 必须正确处理；
- bare repository 可以被识别，但没有编辑工作树功能。

### 8.2 激活策略

- 工作区根本身或其直接父级仓库立即激活；
- 工作区根下的直接仓库可以立即激活；
- 深层嵌套仓库在其文件被打开或目录被展开时激活；
- 不为所有深层目录启动无限制递归 Git 扫描。

### 8.3 Active Repository

默认以当前活动文件所属仓库为 active repository。若无活动文件：

1. 使用 Git 面板中用户最近选择的仓库；
2. 否则使用第一个工作区根关联的仓库。

标题栏、状态栏和 branch picker 均遵循同一 active repository 规则。

## 9. Repository Actor 与一致性

每个仓库拥有一个 actor：

- 写操作严格串行；
- 只读操作有限并发；
- status 等可重复请求按 key 合并；
- 新请求可以替换尚未执行的旧请求；
- 每次扫描生成递增 generation；
- 旧 generation 结果不得覆盖新状态；
- 操作完成后强制刷新仓库快照；
- Git 进程退出、取消或崩溃都必须结束 operation 状态。

刷新来源：

- 打开或恢复 workspace；
- 文件保存；
- 文件系统 watcher；
- Loom Git 操作完成；
- 窗口重新获得焦点；
- 外部工具修改 `.git/index`、`HEAD`、refs 或工作树；
- runtime 或 Git 配置变化。

## 10. 文件监听

需要监听：

- 工作树文件；
- `.git/index`；
- `HEAD`；
- `refs/`；
- `packed-refs`；
- `config`；
- merge/rebase/cherry-pick 状态文件；
- linked worktree 对应的 repository dir；
- common dir 中共享的 refs 和配置。

监听事件需要：

- 150–300ms debounce；
- 合并重复路径；
- 路径级变化优先进行局部 status；
- `.git` 元数据变化触发完整仓库刷新；
- watcher 丢事件、溢出或网络文件系统时回退到完整扫描；
- 窗口重新聚焦时执行一次兜底刷新。

watcher 只负责触发刷新，不能自行推断最终 Git 状态。Git 输出是最终真相源。

## 11. 文档磁盘协调

这是分支切换、merge 和 rebase 的前置基础。

当前 Loom 打开文件后主要维护内存 buffer，需要增加：

```rust
struct DiskState {
    modified_time: SystemTime,
    size: u64,
    content_hash: ContentHash,
}
```

规则：

- 未修改的打开文件发生磁盘变化时自动重新加载；
- 有未保存修改的文件发生磁盘变化时不得自动覆盖；
- 显示“磁盘版本已变化”的明确状态；
- 用户可以比较、保留编辑器版本、加载磁盘版本或合并；
- checkout/rebase/merge 前检查受影响的 dirty buffers；
- 提供保存、放弃、stash、取消操作选项；
- Git 操作结束后统一 reconcile 所有打开文档；
- 文件被删除或重命名后正确更新 tab 和 document path；
- 外部终端执行 Git 操作也必须进入同样的协调流程。

任何会修改 worktree 的 Git 操作都必须经过文档预检。

## 12. Diff 模型

Loom 需要同时处理四层内容：

```text
HEAD → index → disk → editor buffer
```

- HEAD → index：已暂存变更；
- index → disk：未暂存变更；
- disk → buffer：尚未保存的编辑；
- HEAD/index → buffer：编辑器 gutter 和综合预览。

### 12.1 Diff 功能

- unified diff；
- split diff；
- word diff；
- 二进制文件提示；
- 新增、修改、删除行 gutter；
- 上一个/下一个 hunk；
- hunk 展开/折叠；
- project diff；
- commit diff；
- stash diff；
- diff 中继续编辑工作文件。

### 12.2 Hunk 暂存

hunk 或选中行暂存不能依赖交互式 `git add -p`。Loom 需要生成 patch 并通过 stdin 应用到 index。

安全要求：

- patch 必须基于精确的 index/blob 版本；
- 操作前验证 index OID 或 generation；
- snapshot 已过期时刷新并拒绝应用旧 patch；
- CRLF/LF 和末尾换行必须保持正确；
- rename、删除、新文件和首次提交分别处理；
- reverse patch 用于取消暂存时同样进行版本验证。

## 13. Git UI 规划

### 13.1 Drawer 模式

当前右侧 drawer 扩展为：

```rust
enum DrawerView {
    Explorer,
    SourceControl,
}
```

`Ctrl+Shift+G` 打开 Source Control。Clone Repository 保留在欢迎页和命令面板中。

### 13.2 Git Panel

Git Panel 包含：

- repository selector；
- branch 和 detached HEAD 状态；
- incoming/outgoing；
- operation progress；
- commit message 编辑器；
- Conflicts；
- Staged Changes；
- Changes；
- Untracked；
- stash/worktree/remote 入口；
- 每个文件的状态、diff stat 和快捷操作。

支持 flat/tree 两种列表模式，偏好持久化。

### 13.3 文件树

- 文件显示 M/A/D/U/! 等状态；
- 目录聚合子项状态，但不覆盖更严重的冲突标记；
- 状态颜色和字母 indicator 可独立配置；
- Git ignored 文件显示策略可配置；
- 右键菜单提供 Open Diff、Stage、Restore、File History。

### 13.4 标题栏和状态栏

替换当前标题栏硬编码 Git pill：

- 显示活动仓库真实分支或短 SHA；
- dirty、staged、conflict 状态采用独立标记；
- 显示 ahead/behind；
- 点击打开 branch picker；
- 状态栏显示后台 fetch/push/clone 等操作。

### 13.5 Gutter

当前 gutter API 从单一 breakpoint 参数升级为 decoration 列表：

```rust
enum GutterDecoration {
    Breakpoint,
    GitAdded,
    GitModified,
    GitDeleted,
    Diagnostic,
}
```

Git decoration 与未来 breakpoint、diagnostic 不得互相排斥。

## 14. 命令系统

当前 command palette 主要是打开文件列表，需要升级为统一 semantic command registry：

```rust
struct CommandDescriptor {
    id: &'static str,
    title: &'static str,
    category: &'static str,
    enabled: fn(&CommandContext) -> bool,
    execute: fn(&mut CommandContext),
}
```

Git 命令示例：

```text
Git: Clone Repository
Git: Initialize Repository
Git: Open Changes
Git: Stage All
Git: Unstage All
Git: Commit
Git: Amend Commit
Git: Create Branch
Git: Switch Branch
Git: Fetch
Git: Pull
Git: Pull Rebase
Git: Push
Git: Stash
Git: View History
Git: Continue Rebase
Git: Abort Rebase
```

按钮、快捷键、菜单和 command palette 必须调用同一 semantic command，避免产生多套业务逻辑。

## 15. 本地操作

### 15.1 Stage 与 Restore

- stage/unstage 单文件；
- stage/unstage section；
- stage/unstage all；
- stage hunk/selection；
- restore tracked file；
- 删除 untracked file 必须二次确认并说明恢复能力；
- index 发生外部变化时拒绝应用过期操作。

### 15.2 Commit

- commit message textarea；
- 72 字符参考线；
- commit；
- amend；
- signoff；
- signed commit 配置和错误提示；
- hooks 输出；
- 用户姓名/邮箱诊断；
- 提交失败时保留 message；
- commit 成功后提供 undo commit；
- 首次提交和 empty repository 正确处理。

### 15.3 Branch 与 Tag

- 创建、切换、重命名、删除分支；
- local/remote branch 区分；
- checkout remote branch 并建立 upstream；
- detached HEAD 明确展示；
- tag 查看、创建和删除；
- 切换前进行 dirty buffer 和 worktree 预检。

### 15.4 Stash 与 Worktree

- stash all/tracked/staged；
- stash list；
- stash diff；
- apply/pop/drop；
- worktree list；
- 创建新 worktree；
- 从现有分支、新分支或 detached commit 创建；
- 删除 worktree 前检查修改和正在使用状态。

## 16. History 与 Blame

### 16.1 Commit History

- graph 分页加载；
- branch/tag/ref decoration；
- author、时间、subject；
- commit details；
- commit diff；
- compare commits/branches；
- search commits；
- shallow repository 状态和 unshallow 入口。

### 16.2 File History

- 跟踪文件重命名；
- 选择 commit 打开文件 diff；
- 从 Explorer、Git Panel 和 tab 打开；
- 支持 deleted/renamed file。

### 16.3 Blame

- 当前行 inline blame；
- gutter blame 模式；
- hover 查看 commit；
- blame 请求按可见区域和文件版本缓存；
- 文件变化后取消旧请求。

### 16.4 性能

- 使用分页 log；
- 使用长生命周期 `git cat-file --batch-command` 读取 commit/blob；
- 不一次加载完整历史；
- 大结果流式解析并支持取消。

## 17. 远程操作与认证

### 17.1 Remote 操作

- remote 列表和选择；
- fetch；
- pull；
- pull --rebase；
- push；
- set upstream；
- force-with-lease；
- 不默认暴露裸 `--force`；
- clone/fetch/push 显示进度和可取消状态；
- incoming/outgoing commits。

### 17.2 Askpass

建议复用 Loom 主程序的隐藏模式：

```text
git / ssh
    │ GIT_ASKPASS / SSH_ASKPASS
    ▼
Loom --git-askpass <ipc-id>
    │ Named Pipe / Unix Socket
    ▼
Loom 原生认证对话框
```

要求：

- secret 不进入命令行参数、普通日志或持久化 session；
- 认证取消必须终止等待中的 Git 操作；
- 支持 username/password、token、SSH passphrase 和 yes/no prompt；
- 优先与现有 credential helper 协作；
- Loom 不自建长期明文凭据数据库；
- 若需要持久化，使用操作系统安全存储或标准 helper。

### 17.3 网络错误分类

至少识别：

- DNS/网络不可达；
- 代理认证；
- TLS/CA 失败；
- SSH host key；
- SSH key/passphrase；
- HTTPS credentials；
- permission denied；
- non-fast-forward；
- protected branch；
- remote not found；
- remote helper 缺失；
- LFS 缺失或失败。

失败信息需要给出下一步行动，而不是仅展示 stderr。

## 18. 冲突与恢复

### 18.1 冲突模型

读取 index stage：

- stage 1：merge base；
- stage 2：ours/current；
- stage 3：theirs/incoming。

### 18.2 Merge Editor

- Base、Current、Incoming、Result；
- Accept Current；
- Accept Incoming；
- Accept Both；
- 手动编辑 result；
- 冲突块导航；
- 已解决状态检测；
- 保存并 stage。

### 18.3 操作生命周期

- merge continue/abort；
- rebase continue/skip/abort；
- cherry-pick continue/skip/abort；
- revert continue/abort；
- Loom 重启后重新识别进行中的操作；
- commit message 和操作 UI 能恢复；
- abort 后重新协调磁盘与打开 buffer。

## 19. Managed Git 打包与更新

### 19.1 发行目录

建议结构：

```text
Loom/
├── Loom.exe / Loom
├── runtime/
│   └── git/
│       ├── bin/
│       ├── libexec/git-core/
│       ├── ssl/
│       ├── templates/
│       ├── VERSION
│       └── MANIFEST.json
├── THIRD_PARTY_NOTICES.md
└── licenses/
```

不得只复制单个 `git.exe`；clone/push 需要 remote helpers、TLS、SSH 和其他运行时组件。

### 19.2 Manifest

Manifest 至少包含：

- Git 版本；
- 上游来源；
- 平台和架构；
- 文件 SHA-256；
- 构建时间；
- capability；
- license 文件列表。

### 19.3 启动校验

- 定位 runtime；
- 校验必要文件；
- 执行 `git --version`；
- 检查 remote HTTPS/SSH helper；
- 缓存 capability；
- 校验失败时切换到 system Git 或显示修复入口。

### 19.4 更新策略

- Git runtime 版本在构建配置中固定；
- 跟踪 Git、curl、OpenSSL、OpenSSH 和 credential helper 安全更新；
- runtime 可随 Loom 更新；
- 后续可以拆分为独立签名 runtime 更新；
- 不允许工作区目录中的 DLL/helper 覆盖 Loom runtime；
- 保留上一个 runtime 版本以支持失败回滚。

## 20. 安全边界

- 不使用 shell 拼接 Git 命令；
- 所有 pathspec 使用 `--`；
- 后台读取禁用外部 diff、pager 和交互 prompt；
- 对不受信任工作区禁用危险 protocol、external diff、fsmonitor 和隐式 credential 调用；
- hooks 仅在用户明确触发 commit/merge 等操作时运行；
- 显示并处理 `safe.directory`；
- 不在日志中记录秘密；
- destructive operation 展示目标、影响和可恢复方式；
- 对 restore、clean、reset、drop stash、delete branch 等操作实施不同等级确认；
- runtime 和更新产物必须经过签名或哈希验证；
- 第三方许可证和源代码义务在发布流程中自动校验。

## 21. 配置与持久化

可持久化：

- Git panel 位置和宽度；
- flat/tree 模式；
- group by staging/tracked；
- diff view style；
- gutter/blame 偏好；
- 最近选择的 repository；
- runtime 来源和 custom path；
- fetch/pull 默认行为；
- commit message 参考线。

不得持久化：

- 当前 status snapshot；
- token、密码、passphrase；
- 临时 askpass IPC 信息；
- 可能已经失效的 index/blob 内容。

启动时必须重新发现和扫描仓库。

## 22. 分阶段实施计划

### 阶段 0：基础设施与 Managed Git

实现：

- 新建 `src/git`；
- `GitRuntimeManager`；
- `GitBackend` 和 `CliGitBackend`；
- 统一子进程、取消、输出流和错误类型；
- runtime manifest；
- 官方发行包加入 managed Git；
- 现有 clone 迁移到 backend；
- 外部文件变化协调基础；
- command registry 基础。

验收：

- 没有系统 Git 时能够 clone；
- Git 命令不阻塞 UI；
- 中文和空格路径可用；
- managed Git 不修改系统 PATH 或注册表；
- runtime 不完整时能够诊断和回退。

### 阶段 1：只读仓库状态

实现：

- 仓库发现；
- GitStore；
- repository actor；
- porcelain v2 parser；
- watcher；
- branch、ahead/behind；
- Git panel 只读版本；
- 文件树 decoration；
- 真实标题栏和状态栏。

验收：

- 外部终端操作可以自动反映；
- 多根和嵌套仓库状态不混淆；
- 所有 XY 状态正确；
- render 路径无 I/O；
- 旧扫描结果不会覆盖新状态。

### 阶段 2：文件级写操作和 Commit

实现：

- stage/unstage file/section/all；
- restore；
- commit editor；
- commit、amend、undo commit；
- identity/hook/signing 错误；
- operation log；
- document preflight。

验收：

- 首次提交可用；
- hooks 失败时 message 保留；
- index 外部变化不会被覆盖；
- dirty buffer 不会丢失。

### 阶段 3：Diff 与 Hunk

实现：

- HEAD/index/disk/buffer 四层 diff；
- unified/split/word diff；
- gutter；
- project diff；
- hunk/selection stage；
- hunk restore；
- stale patch 保护。

验收：

- 未保存 buffer 即时更新 gutter；
- CRLF/LF 和末尾换行正确；
- 旧 diff 无法修改新 index；
- 大文件 diff 不阻塞 UI。

### 阶段 4：Branch、History、Stash、Worktree

实现：

- branch/tag；
- commit graph；
- commit/file history；
- blame；
- stash；
- worktree；
- `cat-file --batch-command`。

验收：

- branch switch 不丢失编辑；
- 大历史分页加载；
- stash/worktree 后 UI 与磁盘一致。

### 阶段 5：Remote 与 Authentication

实现：

- fetch/pull/push；
- remote selector；
- incoming/outgoing；
- askpass IPC；
- HTTPS/SSH；
- progress/cancel；
- 网络错误分类。

验收：

- 用户无需打开终端完成私有仓库操作；
- secret 不出现在日志和命令行；
- 取消认证能够结束 Git 任务；
- non-fast-forward 等失败可恢复。

### 阶段 6：Conflict 与进行中操作

实现：

- merge editor；
- merge/rebase/cherry-pick/revert 状态；
- continue/skip/abort；
- 重启恢复；
- reflog 辅助恢复。

验收：

- 重启 Loom 后可以继续 rebase；
- abort 后 buffer、磁盘和 index 一致；
- 冲突解决不会静默覆盖内容。

### 阶段 7：高级与完善

实现：

- submodule；
- Git LFS；
- sparse checkout；
- signed commits；
- interactive rebase；
- cherry-pick/revert UI；
- compare branch/commit；
- hosting provider 层。

验收：

- 高级功能不破坏核心 GitStore 语义；
- 未支持的场景提供清晰降级和终端入口；
- runtime capability 不足时正确隐藏或禁用功能。

## 23. 测试计划

### 23.1 单元测试

- porcelain v2 全部 record 类型；
- XY 状态组合；
- rename/copy；
- conflict；
- unborn/detached HEAD；
- ahead/behind；
- NUL 分隔；
- 中文、空格、换行和非 UTF-8 路径；
- diff/hunk 行映射；
- CRLF/LF；
- 错误分类；
- runtime 解析和 capability；
- generation 与过期结果拒绝。

### 23.2 集成测试

每个测试使用临时仓库和 repository-local identity，不读取或修改用户全局 Git 配置。

- init 和首次提交；
- stage/unstage；
- hunk stage；
- rename/delete；
- merge conflict；
- rebase continue/abort；
- stash；
- worktree；
- submodule；
- 外部 CLI 与 Loom 并发；
- index.lock；
- 进程取消；
- runtime 缺失/helper 缺失；
- dirty buffer + checkout；
- Loom 重启恢复进行中操作。

### 23.3 UI 测试

使用 fake backend 和固定 snapshots：

- Git panel 分组；
- repository selector；
- operation enabled/disabled；
- diff 导航；
- commit error；
- askpass；
- merge editor；
- progress/cancel；
- runtime diagnosis。

### 23.4 性能和压力测试

- 100k tracked files；
- 10k changed files；
- 100k commits；
- 大型 monorepo；
- 高频文件变化；
- watcher overflow；
- 网络文件系统；
- 大 diff 和二进制文件；
- 反复取消和重新发起操作。

性能硬性原则：

- 主线程不执行 Git 或文件系统扫描；
- UI 渲染复杂度不与完整仓库文件数直接绑定；
- 列表和 diff 只渲染可见区域；
- 长任务可观察，并在安全时可取消；
- 内存缓存有容量和生命周期上限。

### 23.5 发行包测试

CI 解压最终产物后，在系统 Git 不可见的环境中执行：

```text
git --version
init
clone
status
add
commit
branch
diff
fetch/push 到本地测试 remote
```

验证对象必须是最终打包产物，而不是构建机环境。

## 24. CI 与发布改造

当前 CI 和 release workflow 只打包 Loom 二进制，需要增加：

- 固定 runtime 版本配置；
- runtime 下载或构建；
- 上游校验和验证；
- runtime manifest 生成；
- third-party license 收集；
- 打包完整 runtime；
- clean-environment smoke test；
- x64/ARM64 矩阵扩展；
- runtime 漏洞更新流程；
- 产物内容清单；
- 发布前自动检查 Git/helper 版本。

短期沿用当前平台矩阵：

- Linux x64；
- Windows x64；
- macOS ARM64。

后续根据 Loom 正式支持范围增加 Windows ARM64、macOS x64 和 Linux ARM64。

## 25. 现有文件改造映射

| 文件 | 计划改造 |
| --- | --- |
| `src/main.rs` | 注册 Git 模块、图标和隐藏 askpass 入口 |
| `src/app.rs` | 创建 GitService/GitStore state，布局 Git panel 和对话框 |
| `src/state.rs` | 增加 DrawerView 和 Git UI 状态，不持有运行时对象 |
| `src/workspace_actions.rs` | clone 迁移到 Git backend |
| `src/model/workspace.rs` | 外部文件变化、reload、rename、delete、disk state |
| `src/ui/titlebar.rs` | 使用真实 branch、dirty 和 ahead/behind |
| `src/ui/sidebar.rs` | 文件和目录 Git decoration |
| `src/ui/statusbar.rs` | branch、sync 和操作状态 |
| `src/editor/gutter.rs` | 支持多种 decoration 和 Git line state |
| `src/editor/editor_view.rs` | 保存刷新、外部变化和 buffer diff |
| `src/input/keymap.rs` | Source Control 和 Git semantic actions |
| `src/ui/command_palette.rs` | 升级为通用 command registry |
| `src/workspace_persistence.rs` | 保存 Git UI 偏好，不保存仓库快照和秘密 |
| `.github/workflows/ci.yml` | runtime 获取、校验、打包和 smoke test |
| `.github/workflows/release.yml` | 发布 managed runtime、许可证和 manifest |

## 26. 完成标准

Git 集成达到完整可发布状态时，必须同时满足：

- 用户没有安装系统 Git 时仍可直接使用所有基础 Git 功能；
- 外部 Git 与 Loom 操作可以实时互相反映；
- checkout、merge、rebase 不会丢失未保存内容；
- HEAD、index、disk、buffer 四层状态清晰且一致；
- 本地、远程和冲突工作流无需终端；
- 所有长操作有状态、日志和合适的取消能力；
- 认证秘密不会泄漏；
- 多仓库、worktree、submodule 和特殊路径正常；
- 大仓库不会阻塞 UI；
- managed Git 可以诊断、更新和回退；
- destructive operation 有明确影响说明和恢复路径；
- 最终发行包在无系统 Git 的干净环境通过 smoke test；
- 终端保留为未覆盖高级命令的逃生通道，而不是日常功能的必要补充。

## 27. 实施顺序原则

必须按以下依赖顺序推进：

```text
Managed Runtime / Backend
        ↓
Repository Discovery / GitStore
        ↓
Document Disk Reconciliation
        ↓
Status / Stage / Commit
        ↓
Diff / Gutter / Hunk
        ↓
Branch / History / Stash / Worktree
        ↓
Remote / Authentication
        ↓
Conflict / Rebase / Recovery
        ↓
Advanced Features / Hosting Providers
```

禁止为了快速展示 UI 而跳过 runtime、GitStore、文档磁盘协调或过期状态保护。前三层决定后续功能是否可靠，阶段 3 的 diff/hunk 完成后，Loom 才真正具备成熟编辑器级 Git 体验的基础。
