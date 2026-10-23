# Production Markdown evidence

The Windows x64 JSON was recorded on 2026-10-23 using the production release
runtime, 65636 bytes of source, warm fonts, nine first-readable/edit samples and
30 UI projection + software Skia draw samples. Nearest-rank p95 is reported.
It is an offscreen UiSession measurement, not OS input/compositor latency,
screen-reader coverage, process RSS or native transient heap measurement.

The inspected [rich screenshot](../../docs/images/markdown-preview.png) comes
from that same production UI scene. Exact reproduction and cache/lifecycle
boundaries are in [the architecture](../../docs/architecture/markdown.md).
Three-platform CI uploads separate PNG/JSON artifacts; do not substitute this
Windows observation for Linux or macOS measurements.
