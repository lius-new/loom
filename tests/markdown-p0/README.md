# Historical Markdown P0 evidence

The JSON files and native PNG in this folder record the initial 2026-10-23
technical probes. They are historical measurements, not current product
performance or cross-platform acceptance. The duplicated prototype parser,
view/session models and probe manifest were removed after migration.

Production rules are tested directly, without GUI dependencies:

```sh
cargo test --locked -p loom-markdown-tests
cargo test --locked -p loom-workspace-tests
```

Native shaping, file/memory hosts, input and resources use the production
workspace tests. See [the architecture](../../docs/architecture/markdown.md)
for ownership, budgets and the opt-in production screenshot/performance run.
The three-OS CI runs that same production evidence test and uploads its results.

The old parse baseline used release, two warmups and nine samples; the native
baseline used dev, 420 logical pixels and Microsoft YaHei/Segoe UI on Windows
with Rust 1.98.0. Rust allocation numbers exclude native heaps and RSS. The old
native PNG bypassed the GUI scene; current production screenshots include it.
