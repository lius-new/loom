# Loom 阶段 2 收尾与验收报告

2026-10-13 收尾。项目 C:\Users\yys\loom；基础提交仍为 5ade2963ac1f2046b7ac54f6d2c58d5bf724bf45。阶段 0、1、2 的改动均保留在工作区，未提交。阶段 2 的 R2.1–R2.6 本机自动验收完成，停在此检查点；阶段 3 未开始。

## 完成项目

| 任务 | 完成内容 | 验收 |
| --- | --- | --- |
| R2.1 Search 类型和端口 | features/search 中建立 SearchRequest/SearchResults、Matcher、私有 SearchModel、SearchSourcePort 和 service；实际读盘/ignore 遍历在 infrastructure/search_filesystem | model/service 无 lgui、AppState、Workspace 或直接文件 I/O 依赖；内存端口测试通过 |
| R2.2 独立 UI 状态 | query/include/exclude 的 InputState、IME 草稿、展开、滚动、选中和焦点归入 SearchUiState | 保留跨面板搜索状态，IME 草稿不启动搜索；原有 UI、焦点和输入测试通过 |
| R2.3 任务协调 | app/search_tasks 管理 200ms debounce、取消和带 generation 的任务票据；模型按 generation 与完整 SearchKey 拒收过期结果 | 可控 199/200ms 时间、快速替换、取消、旧成功/失败晚到与启动失败测试通过 |
| R2.4 导航意图 | 私有模型生成 NavigationIntent；feature/navigation 用通用文档快照或注入磁盘源定位；app/search 执行打开、预览、选区和焦点 | 原有脏 buffer、Unicode 选区及旧结果导航测试保留；新增意图/过期行测试通过 |
| R2.5 通用文档快照 | domain/document_snapshot 提供不可变文本快照；Workspace 改为 text_snapshots/dirty_text_versions，移除 Search 专属 DTO 返回值 | 文档共享、dirty/undo、多 pane 和持久化回归通过；hash 算法保持原样 |
| R2.6 消除旧根访问 | Search view 只接收 StateAccess<SearchPanelState>、语义 Commands、根目录显示信息和由 app 组合的 header；不接收根状态、编辑器或 application handle | Search 功能和 UI 源码均无 AppState/AppRuntime/Workspace 访问，Search 目录不允许旧例外 |

本轮按验证后的固定清单更新 23 个文件、新增 19 个文件，零文件删除；逐个校验修改前 hash，并先保存旧文件备份。旧 src/search.rs、src/search_actions.rs、src/ui/search_panel.rs 保留为迁移说明，不再参与模块编译。此前的阶段 0、1 改动、依赖、Git 元数据和用户配置均保留。

## 当前职责与依赖

~~~text
app/search.rs                    应用接线、搜索 key、状态投影、命令、文档事务
app/search_tasks.rs              debounce、取消票据与可注入执行器
features/search/
  types.rs                      请求、结果、匹配数据
  matcher.rs                    正则/glob、Unicode 预览、过期行定位
  model.rs                      私有业务状态与生命周期转换
  ports.rs                      文件源契约
  service.rs                    有界扫描、取消与结果发布
  navigation.rs                 解析打开意图，不操作 Workspace
ui/search/
  state.rs                      输入、IME、焦点与 viewport 状态
  commands.rs                   本地选中/滚动与语义命令回调
  view.rs                       绘制和交互，无根状态依赖
infrastructure/search_filesystem.rs  真实路径、ignore 遍历、受限读取
domain/document_snapshot.rs     通用不可变快照和原 content_hash 算法
ui/components/state_access.rs   同步窄投影；只更新指定值
~~~

方向为 app 组合 UI、feature 和 infrastructure；Search feature 只依赖自身、通用 domain 和匹配库；基础设施实现 SearchSourcePort；UI 只读搜索展示状态并发出语义命令。原来的 Workspace 原子更新和共享 document/pane 事务保持在应用层调用的同一次 State 更新中。

新增架构规则拒绝 Search 中的根状态/GUI/直接 I/O 依赖、Search 的迁移例外，以及 Workspace 重新返回 Search 专属快照。门禁检查 146 个 Rust 源文件、118 个已登记根字段，旧依赖文件从 92 减为 89，Search 功能/UI 无例外。只调整了明确的应用、基础设施和测试调用目标；没有整体刷新旧债务。

SearchModel 状态字段私有，对外返回不可变 SearchStatus。SearchPanelState 仍作为 app 的一个组成字段，组合业务 model 和 UI state；本轮没有把整个 AppState 拆成多个 reactive store。StateAccess 在 lgui State::get 之后投影，仍存在旧根克隆成本，不能声称已彻底解决所有渲染/克隆性能问题。通用 input 的原 State<T>/InputOptions 接口保留，搜索使用新投影接口；其他表单没有被迫迁移。

## 最终验证

| 检查 | 结果 |
| --- | --- |
| 架构门禁和负例自测 | 通过，10 项自测通过 |
| cargo fmt | 通过 |
| Clippy，全部目标，现有 CI 参数 | 通过，0 诊断，无新增豁免 |
| 默认环境 debug 完整测试 | 582 通过，0 失败，12 忽略 |
| 独立 Git 配置 debug 完整测试 | 582 通过，0 失败，12 忽略 |
| release 完整测试 | 582 通过，0 失败，12 忽略 |
| 四项真实 Windows ConPTY smoke | 另行通过 |
| 正式 Loom release 构建 | 通过 |
| 隔离配置实际启动 | 运行 4 秒、根 effect 保存隔离配置，正常关闭，exit=0，无强制清理 |
| 最终检查/采样命令 | 48/48 exit=0 |
| 最终源码 | 与采样 inventory 的 hash 全部匹配 |
| Cargo.toml / Cargo.lock | 与阶段 0、1 hash 相同 |

相比阶段 1 的 568 项默认通过测试，新增 14 项：9 项纯 Search 模型/服务/意图验证，4 项任务时序/失败验证，1 项投影更新验证。原有搜索匹配、ignore/hidden、glob、正则、限制、UI、键盘和文档导航测试继续运行。12 项 ignored 的登记保持原样，其中性能和原生终端项目已单独执行。

文件上限 4 MiB、匹配上限 10,000、文件结果上限 5,000，ignore/hidden/.rgignore/.git 规则、60ms 进度发布、200ms debounce、预览/永久打开和脏 buffer 优先规则保持原逻辑。启动 smoke 只使用临时子进程 APPDATA，关闭本次创建的 PID 16288，未关闭已有 Loom 进程或修改用户真实会话。正式应用 SHA256 为 8F3B399793DDA3B0FD8A133B168E7D8B69836AB41CD09D644F63E4FD7DBB3CB4。

## 性能

六组 release 性能测试各一次预热加五次正式样本，共 31 指标、155 条记录，无解析 notices。与阶段 1 的同机历史中位数相比，没有指标上升超过 10%；最大计时上升为 24x80-screen/terminal_screen_snapshot_100 的 9.70%。

| 搜索指标 | 阶段 1 中位数 | 阶段 2 中位数 | 变化 |
| --- | ---: | ---: | ---: |
| search_scan_cached | 27.079100 ms | 26.307800 ms | -2.85% |
| search_cancel_latency | 0.181100 ms | 0.077100 ms | -57.43% |

完整数据和原始日志保留在此目录。历史采样不是交错 A/B，不宣称性能优化；UI session 测量仍不等于原生窗口帧延迟、输入分位数或内存峰值。本轮没有出现需要再次对照的超过容差指标。

## 下次接续

本阶段完成并停止推进。下次从阶段 3 的 R3.1 开始：先按既定计划提取独立编辑核心，确认 identity、text、selection 与 Vim 的依赖边界，再建立独立核心测试入口；不在本次收尾中开始这些改动。

阶段 0 留下的原生 IME/候选窗口、视觉与系统焦点、macOS/Linux、远端 CI 及安装包验收仍待执行。当前本地绿色结果不替代这些验收。原始阶段 0、1 的报告与数据保留，当前进度见 docs/architecture/progress.md。
