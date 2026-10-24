# Loom Git 工作台原型

使用 Tailwind CSS 4 构建的本地 HTML 交互原型，无需连接真实 Git 仓库。已包含编译后的样式；可以直接打开 `index.html`，不依赖 CDN。

更推荐通过本地服务器预览：

```powershell
node docs/prototypes/git-workbench/server.mjs
```

浏览器访问 http://127.0.0.1:4317 。最低设计宽度为 720px，建议 1440×900 或更大；窗口不超过 1100px 时 Diff 自动使用统一视图，历史详情栏隐藏。

支持文件筛选和预览、文件/块级暂存、并排/统一 Diff、模拟提交与修订（含“提交并推送”下拉）、拉取/推送预览、分支切换、提交历史详情以及 Stash/Worktree 信息查看。

覆盖的状态：冲突文件（需先“标记为已解决”才能提交）、未跟踪文件、工作区干净的空状态、拉取/获取的加载态（每第 3 次获取模拟失败以展示错误提示）。Git 面板高度可拖动（双击恢复，方向键微调）。底部状态栏左侧只放 Git 面板开关。所有状态仅保存在内存中，刷新即恢复；本原型没有真实 Git 操作。

修改 Tailwind 类后重新构建：

```powershell
cd docs/prototypes/git-workbench
npm ci
npm run build
```
