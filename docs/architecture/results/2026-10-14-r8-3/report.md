# R8.3 文件服务与窗口适配验收

2026-10-14，起点 93f0d02d26b05de07ce01d110684db8689dd9b75，工作目录 C:\Users\yys\loom，分支 refactor/r8-chrome。R8.3 本机自动验收完成，继续 R8.4，R8 整体仍在进行。

infrastructure/file_loader 持有 work/timer 队列与原子代次，接收泛型请求元数据和 app 注入的准备/完成/延迟回调，不持 AppState 或 UI。app/file_loading 保留 pending 请求、目标 pane/活动文档/根目录检查和一次工作区提交；延迟仍为 300ms，突发 coalesce 与取消代次保持。交付时再核对弱 lease，已关闭服务的完成和延迟通知拒收。worker 清理保持原异步信号，不在 UI 线程新增 join。

infrastructure/file_watcher 持有原生/polling watcher、quiet window、批次合并及资源停止；app/file_watcher 保留 Explorer/文档/search 跨模型对账、pointer/version/根目录校验与重试。生产回调携带取消 lease 到 UI 队列和重试，关闭或替换 watcher 后不发布旧结果/错误。新增两项真实资源/队列回归验证：保留模型快照时 shutdown 后已排队文件交付拒收、真实 watcher 关闭后已排队对账不得覆盖原文档。

64 MiB 有界 UTF-8 读取、文档写入和系统文件选择归 infrastructure；打开/保存/脏文档关闭流程与会话协调在 app。WindowGeometry 为纯 domain DTO；窗口位置/尺寸/最大化/当前进程窗口探测及命令行路径记录归 infrastructure，配置 load/save 与应用关闭事件归 app/window。窗口 JSON 字段、保存锁和退出顺序保持；原有 root 名称仅为后续 R8.5 清理的接缝，不宣称别名/Root runtime 已结束。

完整 workspace **715 通过、0 失败、15 ignored**（应用 618、编辑核心 68、域重复验证 29）。631 个原应用入口逐项映射无缺失，28 个源码测试移动，新增 2 项，声明 700→702，旧 ignored 不变。54 Node 自测（架构 51、报告 3）、严格门禁、all-targets、fmt、CI Clippy、release 与隔离启动通过。260 Rust 文件，19 根字段、12 遗留例外、10 根 UI 适配器。15 项原生读取/合并/探测/启动源码对照通过。

release SHA-256 E529FAECF1F18E88F59B6CE7EBF26EAB84C20847ED3E9C37E227E126CAF918F4；隔离启动 PID 21648 正常退出 0，无强制清理。首次测试导入、窗口 event 重复接线和 Clippy import 位置问题已修复，失败日志保留；测试和原生对账只操作合成临时文件，不关闭用户进程或修改真实配置。

原始记录 target/baseline/2026-10-14-r8-3；Cargo/lock/toolchain 未改，实际 Rust 1.98.0。R8.4–R8.6、最终性能和独立 Git/release 全量复验，以及固定 1.97.1、原生人工、Linux/macOS、远端 CI 和真实安装仍待验。
