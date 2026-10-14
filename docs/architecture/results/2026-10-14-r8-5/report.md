# R8.5 状态与运行时归属验收

AppState 已移除弱 runtime 和访问方法，仅保留 15 个具名组合字段。显式 AppContext 保存 reactive model 和 Weak<AppRuntime>；UI 投影获得 model，不获得应用上下文。取消 pending file 的原生失效由应用事务边界完成。

通知/首页 hover 与 drawer 类型归 app/state/chrome，tab 交互及 diff divider 归 UI；旧根别名与 model/workspace 兼容导出删除。unicode_width 是单行文本测量的纯 UI 依赖，已登记为精确合法依赖，不保留迁移例外。

715 通过、15 ignored；633 个旧应用入口无缺失。59 架构/解析自测、严格 Clippy、fmt、release、隔离启动与 4 项真实 ConPTY 测试通过。R8 例外和根 UI 适配器均为 0。剩余 .runtime() 登记只指框架 UiSession 测试/动画 runtime。

实际 Rust 1.98.0；固定 1.97.1、人工视觉/IME/焦点、跨平台及远端 CI 待验。继续 R8.6 最终回归与冻结二进制性能比较，未合并/推送。
