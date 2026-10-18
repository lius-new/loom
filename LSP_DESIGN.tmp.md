# Loom 通用 LSP 设计（临时草案）

- 日期：2026-10-18。
- 状态：后续 P4 实施草案，未实现。
- 公共身份、坐标、语言注册表和高亮模型以 [主设计](LANGUAGE_SERVICES_DESIGN.tmp.md) 第 4–7 节为准；本文件不重复定义。
- 适用范围：多语言通用客户端。rust-analyzer 是首个真实集成样例，协议核心必须通过不同能力组合和非 Rust 假服务器验证。
- 默认首期：P4a 通道 → P4b 文档同步 → P4c 可见结果。P4d 同步/delta 优化、P4e 通用 watcher、P4f 完整进度单独交付。
- P4 不阻塞主设计的 P1–P3；未完成的 watcher/progress 不得提前声明能力。
- 实施状态：尚未开始；P4 的唯一状态维护入口见 [第 5 节进度跟踪](#5-实施进度跟踪)。

## 1. 通用 LSP 服务设计

### 1.1 首期范围与实例模型

P2 的语言描述只包含识别、显示与 grammar 资源字段。ServerDefinitionId、ServerBinding、协议 languageId 及服务器根策略到 P4a 才加入相应数据结构/描述格式，并同步补严格校验与测试；本稿的概念模型不要求 P2 预留未实现字段。

先支持本机 stdio 服务器、一个服务器配置与一个工作区根目录的实例。实例键为 `(server_definition_id, root_uri, config_generation)`，不以语言名或 rust-analyzer 常量作为实例键。同一个实例可服务多种语言的多个文档，同一文档的多个窗格不会重复 didOpen。

根目录通过 ServerDefinition 中的 RootPolicy 解析：显式根覆盖、工作区根、最近 marker 或具名 provider。marker、优先级与无项目时的策略由描述提供，解析工作在后台执行，不在 UI 线程读目录或运行项目命令。

Rust 示例使用 Cargo.toml marker/provider，在已打开的工作区边界内选择有效 Cargo 项目根；TypeScript 等语言可以选择自己的 marker 集合而无需修改 manager。无 marker 时是否使用文件所在目录由服务器定义决定，并报告实际可用状态，不统一承诺完整项目语义。初期不支持未命名草稿文件的 LSP；保存成文件后接入。非文件 URI 在将来单列虚拟文档支持。

首个真实服务器使用 rust-analyzer，但完整通路必须通过可替换的 ServerDefinition 驱动。任何服务器程序缺失时，状态为 unavailable，继续本地高亮；提供配置位置与重启命令，不弹出重复错误。首期不自动安装。

### 1.2 模块边界与线程

- `features/language_service`：会话状态、能力模型、同步规划、语义解码、诊断模型；不直接启动进程。
- `infrastructure/lsp`：子进程、stdio framing、读写、超时、退出与回收。
- `app/language_services.rs`：将文档事件送入服务，将不可变结果通过 UI 调度器交付。
- `AppRuntime`：持有服务生命周期；UI 仅获取状态与快照。

首期沿用 std 线程与有界消息通道，不单为 LSP 引入全应用异步运行时。每个实例的读通道、写通道与 stderr 消费独立，禁止因阻塞写入导致无人读取响应。所有实例共享一个管理协调器，控制实例数量与请求调度。

本文的“LSP 同步线程”指语言服务管理协调器的后台同步规划执行路径，负责快照差分和同步消息构造；它不在 UI 线程或 syntax_worker 中执行，也不要求为每个文档单独创建线程。协议 I/O 消费保持独立，不因正文差分而停止读写。

读取处理正确支持半包、粘包、按 UTF-8 正文字节数计的 Content-Length。stdout 仅解析协议，stderr 持续消费到有界日志。不要假定读一次得到一条 JSON。

### 1.3 生命周期

```text
Stopped → Starting → Initializing → Ready → Stopping → Stopped
                       └→ Failed ←───────────┘
```

顺序：启动 → initialize → 读取能力与编码 → initialized → 同步待打开文档 → 请求能力结果。

结束时先停止新任务，尝试 shutdown/exit；超过有界等待后终止并回收自有进程。Windows 后台服务器不显示控制台窗口。所有清理幂等；AppRuntime 销毁后排队的回调不得恢复服务或写入状态。

可提出初始策略：初始化超时 30 秒、普通语义请求超时 5 秒、退出宽限 2 秒；这是本设计的可调整默认值，并非协议要求。超时不阻塞编辑。崩溃最多按 1/2/4 秒退避自动重启三次，之后等待显式重启；配置变化重建实例代次。

### 1.4 能力协商与服务端请求

只声明已经实现的能力：文档同步、支持的 position encodings、push diagnostics、semantic full、必要的 workspace/configuration 与 semantic refresh。P4a–P4c 默认不声明客户端文件监视动态注册；首个真实服务器使用经过锁定版本集成测试验证的自监视模式。完成 P4e 的注册/取消、模式匹配和事件投递后，才声明 `workspace.didChangeWatchedFiles.dynamicRegistration`；其他能力仍不宣称支持动态注册。

首批不声明 pull diagnostics、partial results、多行或重叠 semantic token 支持、workspace/applyEdit 或补全能力。P4c 先提供连接与等待结果状态；workDoneProgress 在独立 P4f 实现第 1.9 节并测试后再开启，不能仅靠设置一个 capability 字段假装支持。

必须回应服务器发来的请求，不能只处理客户端发出的请求：

| 请求/通知 | 初版行为 |
| --- | --- |
| `workspace/configuration` | 按请求 item 顺序返回已配置值或 null |
| `workspace/semanticTokens/refresh` | 回应并递增语义代次，重新调度可见文档 |
| `client/registerCapability` / `client/unregisterCapability` | P4e 完成后仅注册/释放已声明的文件监视订阅；首期不声明，不接受未实现的注册 |
| `window/workDoneProgress/create` / `$/progress` | P4f 完成并声明能力后处理 token 与 begin/report/end；此前不承诺进度能力 |
| `window/logMessage` / `window/showMessage` | 有界日志或非重复的状态消息 |
| `workspace/workspaceFolders` | 若已声明支持，返回该实例根；未实现则不声明 |
| `workspace/applyEdit` | 不声明支持；意外收到时明确返回未应用，首期不执行编辑 |
| 未知 request | 按 JSON-RPC 返回 MethodNotFound |
| 未知 notification | 忽略或限流记录，不使实例失败 |

不要发送未协商能力的请求。仅提供 range semantic tokens 的服务器首期显示为该高亮能力暂不可用；后续 range 支持有单独验收，不伪装成 full。

### 1.5 文档同步

为每个服务器实例中的 URI 建立独立、递增的 LSP version，并保存它到 DocumentStamp 的映射，不直接截断核心 u64。同一实例中关闭再打开同一 URI 时也不复用协议版本，保留版本计数或墓碑，避免旧诊断的版本号碰撞。协议整数接近上限时重建服务器实例并增加实例代次，而非简单将同一连接上的版本号归零。

- didOpen 发送最新完整正文。
- didChange 按服务器同步类型执行；同步为 None 时不发送修改，并在本地编辑后禁用该文档语义结果，直到有效重开/同步。
- didSave 只在磁盘保存成功后发送；是否包含正文遵循服务器能力。
- URI/语言变化先关闭旧会话、清理结果，再以新 URI/语言打开。根变化还需要重新路由实例。
- 文档真正关闭后 didClose；只是关闭一个窗格而文档仍打开时不关闭会话。
- 服务器重启后从最新正文重新 didOpen，不重放旧请求。

所有文档通知与该文档请求经同一个实例写队列排序，必须先写入更新再发送依赖该更新的 semantic 请求。状态区分已提交到有序写队列的基准与已完成写入的基准；尚未发送不能当作服务器已确认处理。协议没有逐次 didChange 确认，响应的版本关联仍依赖有序发送与请求上下文。

首个可验收子步骤可以对支持修改同步的服务器发送无 range 的完整替换。之后在 LSP 同步线程中比较 last-sent 与 latest 两份快照，用公共前缀/后缀构造一条连续替换，减少传输；前后缀必须收敛到合法字符边界。这里指语言服务的同步规划执行路径，不是本地语法服务的 syntax_worker。

这条差分不依赖核心 revision 中保留插入正文，也不需要重建所有被合并掉的中间文本。range 用旧快照按协商编码计算，replacement 从新快照取得。对于文本相同但本地版本变化的情况，明确更新本地与协议版本对应关系；最简单的首期行为是发送完整替换以推进协议版本。

合并发生在尚未派生协议消息的目标状态上。不能丢弃任意已经排队的增量 range 后继续使用旧基准。写队列拥堵时丢弃待派生的中间目标，重新从最后已排队的同步基准规划；已进入有序传输的 didOpen/change/close 与依赖请求不得因队列压力静默丢弃。传输写入失败或无法证明顺序完整时使实例失效并重新建立，禁止继续在损坏的镜像上发请求。UI 投递到的是可合并目标邮箱，不直接等待进程写队列腾出空间。

### 1.6 Semantic tokens

第一步只实现 full：从服务器 legend 映射分类；按协议相对编码解码为绝对位置，使用请求绑定的正文快照转换字节范围。验证数据长度、类型索引、修饰符位、整数溢出与文本边界。

每个请求记录：请求 ID、服务器代次、文档打开代次、DocumentStamp、语义失效代次。响应通常不携带核心文档版本，必须由请求上下文绑定，不能把到达时间当作版本证明。

每个文档最多一个有效语义请求在途。新编辑使旧请求过期，并尽力发取消；服务器可以忽略取消，客户端仍必须丢弃旧结果。取消与超时后释放在途槽位，不等待服务器无限响应。

收到 refresh 时，即使文本没变化也递增语义代次，使 refresh 前的在途结果不能覆盖新语义。初版仅在编辑停顿约 75 ms 后发请求；这是可调调度参数，不影响正文同步。切到已有有效快照的文档可直接显示。

第二步再实现 full/delta：保存服务器原始整数数组与 resultId。delta 在原始数组层验证并应用，构造新数组后整体解码；start/deleteCount 是整数数组索引，不是 token 索引。多条 edit 都以同一份旧数组为基准，验证不冲突后按 start 从后向前应用，不能假定服务端已排序。服务端可以返回 full；缺基准、非法 delta、服务器重启时回退 full。

`null` 与空 token 集合都应清除当前发布的语义层；不据此永久判定服务器不支持。未明确返回新 resultId 时，不继续假定旧 delta 基准可用。

之后如有必要增加 range 路径：仅查询当前视口、单独缓存 coverage，不将局部结果冒充全文结果。完成后才声明对应能力。

### 1.7 诊断

首期只处理 push diagnostics，按 URI 与服务器实例存储；每次发布替换该来源先前集合，空集合表示清除。可视交付至少包含编辑器下划线、当前文档计数和查看消息的入口，不在首期建设完整 Problems 面板。

诊断带 version 时，只接受可映射到当前打开代次和当前文本的版本；无法对应的旧版本丢弃。编辑后先清除当前范围装饰，避免位置错位。

诊断 version 是可选字段。缺少版本时无法严格证明它与当前正文对应：在接收时以已发送快照做范围校验，标记为 unversioned；如果服务正在追赶本地修改，不发布范围装饰。即使本地已追平，也只能按未版本化诊断展示，下一次编辑立即失效，不能声称实现了严格的版本一致性保证。

多个实例的诊断按来源分别存储与清理；关闭实例不得清空其他实例的结果。首期只将打开文档的范围诊断送入编辑器，关闭文件的工作区诊断汇总留待后续。

### 1.8 文件监视与工作区外部变更（后置 P4e）

本节通用 watcher 不属于 P4b/P4c 的默认交付范围；在第二个真实服务器或已验证的功能缺口需要时实施。首期服务器自监视必须通过实际测试，不能凭未声明 capability 就认定会自动回退。rust-analyzer 官方配置将 files.watcher 默认值列为 client；具体 fallback/显式 server 模式仍以锁定版本验证为准。

不假定所有服务器自行监视文件。复用现有 notify 文件事件源，通过通用订阅适配器匹配服务器注册的 glob、base URI 与 Create/Change/Delete 类型，生成 `workspace/didChangeWatchedFiles`，不为每种语言重复建立一套 watcher。

订阅按服务器实例代次和注册 ID 管理，取消注册或退出时释放。只有真正实现并验收的 glob 语义与 relative pattern 能力才可声明；不能默认现有 globset 配置与 LSP 模式完全等价。注册范围超出已有监视根时，需要取得受资源预算限制的额外 watcher lease，无法建立则明确报告注册失败，不静默漏报。

合并重复文件事件，但保留创建/删除的最终语义；重命名按旧 URI 删除、新 URI 创建表达。文件系统事实通知与内存正文 didChange 是两条通道：打开且未保存的正文仍以协议同步为准，不因磁盘事件被覆盖。现有干净/脏文档冲突处理继续由 workspace 负责。

现有批次的 `requires_full_rescan` 没有一一对应的 LSP 事件。需要比较受监视范围的已知元数据或安排受控项目重载，不能伪造一个不存在的 Rescan 协议通知。验证大量 git 分支切换、事件丢失、原子保存和目录删除的收敛路径。

验收覆盖未打开的源文件、项目清单（如 Cargo.toml 或其他语言的 marker）、新建/删除文件和 git 分支切换。若首个服务器使用原生监视模式，也必须实际验证，并将选择封装在该服务器适配/配置中；通用客户端仍保持文件事件能力边界清晰。

### 1.9 通用进度与服务状态（后置 P4f）

P4c 先区分连接状态与是否已有结果，没有进度能力时显示“已连接，等待语义结果”。本节完整 token 生命周期、并发任务与百分比属于后续 P4f，可根据用户体验需求在 P4c 后单独实施。

协议连接 Ready 只表示初始化完成。服务状态拆成“连接状态 + 可选后台任务集合 + 能力结果可用性”，连接就绪时仍可存在加载项目、索引或检查任务。

完成 workDoneProgress 能力后，按服务器实例与 token 处理 create/begin/report/end，显示服务端提供的标题、消息与可选百分比，不写死 rust-analyzer 私有 token 或消息字符串。多个任务可并存，实例退出清空；异常丢失 end 需要有界清理与状态提示。

首期可以先提供简洁的“后台分析中”状态，不必实现完整进度面板。只有服务器明确报告可识别的索引任务时显示“索引中”；没有进度时显示“已连接，等待语义结果”，不能把长时间无结果推断成正在索引，也不能承诺固定耗时。

## 2. 阶段范围与首期监视验证门槛

| 阶段 | 工作内容 | 完成标准 |
| --- | --- | --- |
| P4a | 增加服务器绑定格式、通用服务器定义、stdio、初始化、请求与生命周期 | 描述新增字段严格校验；非 Rust 假服务器可配置，分帧/超时/退出正确 |
| P4b | 事务桥接、URI/root、文档同步、版本与 UTF-8/UTF-16 | 不可见文档和所有写入口同步正确；不包含通用 glob watcher 和完整进度实现 |
| P4c | push diagnostics、semantic full、refresh、状态展示 | rust-analyzer 真实 smoke 与差异化假服务器通过；缺能力可回退 |
| P4d | 单范围正文差分与 semantic delta | 镜像与最新正文一致，非法 delta 回 full |
| P4e | 通用文件监视注册、glob、额外根 lease、重扫收敛 | 客户端监视路径独立验收通过后再声明 capability |
| P4f | workDoneProgress create/begin/report/end 与多任务 | token 按实例管理，状态真实、退出清理正确 |

P4b/P4c 的服务器自监视路径是有条件的范围缩减：

1. 记录真实服务器版本/二进制标识、平台、有效初始化配置与是否声明客户端 watcher。
2. 验证未打开源文件的修改、新建、删除，以及项目清单/配置变化能更新分析结果；Rust 至少覆盖 Cargo.toml 和适用的 .cargo 配置。
3. 测试外部磁盘编辑和 git 分支切换，不能只测当前 didOpen 文档的保存通知。
4. 首期声明支持的平台都要覆盖。若自监视存在缺口，先尝试文档化的显式模式；仍失败则将必要 watcher 子集提前或明确限制该适配的支持范围，不能带着已知漏报宣称完成。
5. 上述验证属于 P4 开始前的集成检查，不要求现在安装或启动服务器。当前没有实测证据，默认值与官方说明不替代锁定版本测试。

不要把某服务器的自监视私有选项放进通用协议核心；通过 ServerDefinition/provider 配置。第二个真实服务器需要客户端监视时，可把 P4e 提前到该服务器适配之前，不能为等待 P4e 静默缺失功能。

## 3. 测试与观测

首期协议测试使用可控假服务器，不要求网络或本机安装真实语言服务器。覆盖 framing、能力组合、同步/编码、请求乱序、取消、refresh、诊断版本、URI 与生命周期。真实 smoke opt-in，明确版本、平台和工作区条件。

P4e 的注册、relative patterns、glob、额外根、rename 与 rescan 测试单独分组；P4f 的 token、多任务和缺失 end 清理也单独分组。不因公共测试清单提到了它们，就使其成为 P4b 的隐式前置条件。

记录服务器启动、首次语义结果、热态更新、发送量、排队和故障指标；服务器项目索引内存与 Loom 自身资源单列。未收到进度时使用“等待语义结果”，禁止推断索引完成或固定索引耗时。

## 4. 官方参考资料

- [LSP 3.17：初始化与能力协商](https://github.com/microsoft/language-server-protocol/blob/gh-pages/_specifications/lsp/3.17/general/initialize.md)
- [LSP 3.17：文档修改](https://github.com/microsoft/language-server-protocol/blob/gh-pages/_specifications/lsp/3.17/textDocument/didChange.md)
- [LSP 3.17：semantic tokens](https://github.com/microsoft/language-server-protocol/blob/gh-pages/_specifications/lsp/3.17/language/semanticTokens.md)
- [LSP 3.17：push diagnostics](https://github.com/microsoft/language-server-protocol/blob/gh-pages/_specifications/lsp/3.17/language/publishDiagnostics.md)
- [LSP 3.17：文件监视](https://github.com/microsoft/language-server-protocol/blob/gh-pages/_specifications/lsp/3.17/workspace/didChangeWatchedFiles.md)
- [LSP 3.17：协议模型与进度能力](https://github.com/microsoft/language-server-protocol/blob/gh-pages/_specifications/lsp/3.17/metaModel/metaModel.json)
- [rust-analyzer：配置，包含 files.watcher](https://rust-analyzer.github.io/book/configuration.html#rust-analyzerfileswatcher)

## 5. 实施进度跟踪

最近更新：2026-10-18。P4a 已实现并待验证（假服务器与真实进程已测，真实 rust-analyzer 尚未接入）；其余子阶段未开始。

状态与证据规则沿用 [主设计第 19 节](LANGUAGE_SERVICES_DESIGN.tmp.md#19-实施进度跟踪)。本节是 P4 状态的唯一维护位置；默认先交付 P4a–P4c，后续子阶段按需求启动，不把未启动的可选项算作首期阻塞。

### 5.1 阶段总览

| 阶段 | 当前状态 | 实现/验证证据 | 完成日期 | 下一步或启动条件 |
| --- | --- | --- | --- | --- |
| P4a 通道与生命周期 | 待验证 | R-L1 | — | 缺 Linux/macOS 运行；服务器进程的子进程树清理（Windows Job Object）未做 |
| P4b 文档同步 | 未开始 | — | — | 事务桥接、URI/root、版本、编码与自监视验证 |
| P4c 可见结果 | 未开始 | — | — | 诊断、semantic full、刷新、状态及真实 smoke |
| P4d 同步/delta 优化 | 未开始 | — | — | 首期正确性稳定后按数据决定 |
| P4e 通用文件监视 | 未开始 | — | — | 第二真实服务器或已知自监视缺口需要时启动 |
| P4f 完整进度 | 未开始 | — | — | 按需求接入通用进度 token 与多任务 |

### 5.2 关键检查点

- [x] **P4a-1**：服务器绑定格式在本阶段加入并通过严格校验；非 Rust ServerDefinition 与不同能力组合不修改协议核心即可运行。
- [x] **P4a-2**：半包/粘包、服务端请求、超时、取消、异常退出与幂等清理通过。
- [ ] **P4b-1**：所有正文写入口、不可见文档及多窗格共享会话同步通过。
- [ ] **P4b-2**：URI、根目录、打开代次、协议版本和 UTF-8/UTF-16 转换通过。
- [ ] **P4b-3**：锁定真实服务器版本与有效配置；未打开文件、项目清单和分支变化的自监视已实测。若失败，记录必要 watcher 提前或支持范围调整。
- [ ] **P4c-1**：push diagnostics、semantic full/空结果、refresh 与过期结果丢弃通过。
- [ ] **P4c-2**：连接/等待/结果状态真实；无服务器或能力缺失不影响本地编辑与基础高亮。
- [ ] **P4c-3**：opt-in 真实服务器 smoke 及差异化假服务器验证有环境、版本和结果记录。
- [ ] **P4d-1**：正文差分镜像一致，semantic delta 的数组索引、无序 edits 和非法结果回 full 通过。
- [ ] **P4e-1**：监视注册/取消、glob、relative pattern、额外根 lease、rename/rescan 收敛通过后才声明能力。
- [ ] **P4f-1**：progress create/begin/report/end、并发任务和实例退出清理通过后才声明能力。

### 5.3 证据与阻塞记录

当前无已确认的实施阻塞。更新时沿用主设计的记录模板，标注本节检查点 ID；真实服务器测试必须记录二进制/版本、平台、配置、工作区条件及结果。假服务器通过不能替代真实自监视验证。

| 日期 | 变更记录 | 实施影响 |
| --- | --- | --- |
| 2026-10-18 | 建立 P4 子阶段与关键验收跟踪 | 文档更新；未启动协议实现或真实服务器测试 |

| 2026-10-18 | 实施 P4a：服务器定义数据、协议核心、进程客户端与假服务器（R-L1） | 尚未接入应用；Rust 绑定到 rust-analyzer 但还没有任何文档触发它 |

记录编号 / 日期：R-L1 / 2026-10-18
阶段 / 检查点 ID：P4a / P4a-1、P4a-2
状态：待验证
实现证据：
- 服务器定义格式：assets/servers/<name>.json（程序、默认参数、项目根规则：marker 文件名、nearest 或 outermost、回退为 workspace、file_directory 或 none，可选 initialization_options）；语言变体用 `server: { definition, language_id }` 绑定，language_id 是服务器自己的词汇，与 Loom 的语言名无关。构建脚本内嵌 assets/servers，注册表严格校验（未知服务器、重名、空 marker、非对象 options、未知字段）并按排序名称分配 ServerDefinitionId；Rust 绑定到 rust-analyzer。内嵌表与磁盘文件逐项核对的测试同样覆盖该目录。
- 协议核心（src/features/language_service，无 I/O）：framing.rs（Content-Length 以字节计；任意切分、粘包、一字节一读、大小写与多余头、重复或非法长度、超限在读取正文前拒绝、头部过长）、message.rs（请求、通知、响应与错误的解析与序列化，null 结果与无 id 错误响应）、capabilities.rs（位置编码 utf-8 或 utf-16，其余拒绝；同步方式数字或对象；semantic tokens 的 legend 与 full、delta、range；客户端只声明已实现的能力）、session.rs（握手、initialized、请求与取消与超时、服务端请求一律有应答、关停、崩溃与幂等清理的状态机）。
- 进程客户端（src/infrastructure/lsp/client.rs）：读、写、stderr 与驱动四个线程，写线程独立所以对端不读也不会卡住驱动；初始化 30 s、请求 5 s、退出宽限 2 s 的默认超时（可配）；Windows 上不弹出控制台；超时的请求通知服务器取消并立刻向调用方返回错误，之后到达的应答被丢弃；崩溃使所有等待中的请求失败并给出退出码；关停先 shutdown 再 exit，宽限期过后强制结束；Drop 结束进程。
- 假服务器：examples/fake_lsp.rs（独立进程，可按参数改变编码、同步方式、semantic 模式、初始化失败或延迟、崩溃、忽略请求、分块或粘包写入、超大消息、坏帧、stderr 输出、拒不退出等，并把收到的每条消息记入日志文件）。
- 架构门禁：协议核心只可用 serde_json；进程客户端可用 std::process 与 std::thread，其测试可读日志文件；各有自测。
验证：cargo test --locked --workspace 全部通过（loom 830，其中协议核心 34 项、进程客户端 17 项）；进程测试连续重复运行稳定；clippy（CI 同参数）、fmt、架构门禁与 61 项自测通过。覆盖：5 种服务器能力组合的握手、参数与初始化选项到达服务器、非 ASCII 负载往返、一字节写与粘包、拒绝初始化与不支持的编码、初始化超时并结束进程、忽略的请求超时后会话继续并发出取消、崩溃（请求中与初始化后）、有序关停并回收进程、不回复 shutdown 与不退出的服务器被结束、Drop 结束进程、程序不存在、stderr 保留、服务端请求被应答并送达（配置按项、动态注册被拒）、无法解析的消息被跳过、坏帧与超大消息使会话失败、会话结束后命令被拒、仅由数据定义的非 Rust 服务器无需任何语言专用代码即可运行（其参数与 language_id 取自数据）。
未完成：Linux/macOS 上运行；服务器子进程树的清理（强制结束时 rust-analyzer 派生的进程可能残留，计划在 P4b 以 Windows Job Object 与进程组处理）；settings 里的 language_servers 配置、应用接入与文档同步属 P4b。
下一步：P4b（事务桥接、URI 与根目录解析、文档同步、位置编码转换）。
