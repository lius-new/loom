//! A scriptable stand-in for a language server, used by the LSP client's tests.
//!
//! It is a real process that speaks the protocol over stdin and stdout, so the
//! tests exercise pipes, threads and process control without needing rust-analyzer.
//! Flags choose how it behaves; see `main` for the list. Every message it receives
//! is appended to the file named by `--log` as one JSON object per line.
use serde_json::{Value, json};
use std::fs::OpenOptions;
use std::io::{BufRead, Write};
use std::time::Duration;

struct Options {
    args: Vec<String>,
}

impl Options {
    fn flag(&self, name: &str) -> bool {
        self.args.iter().any(|a| a == name)
    }
    fn value(&self, name: &str) -> Option<&str> {
        let at = self.args.iter().position(|a| a == name)?;
        self.args.get(at + 1).map(String::as_str)
    }
    fn number(&self, name: &str) -> u64 {
        self.value(name).and_then(|v| v.parse().ok()).unwrap_or(0)
    }
}

fn read_message(input: &mut impl BufRead) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0; length?];
    input.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

struct Output<'a> {
    options: &'a Options,
}

impl Output<'_> {
    fn frame(body: &Value) -> Vec<u8> {
        let body = serde_json::to_vec(body).unwrap();
        let mut frame = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        frame.extend(body);
        frame
    }

    fn write(&self, bytes: &[u8]) {
        let mut out = std::io::stdout().lock();
        let chunk = self.options.number("--chunk") as usize;
        if chunk == 0 {
            let _ = out.write_all(bytes);
            let _ = out.flush();
            return;
        }
        // One small piece at a time, so the client sees headers and bodies split anywhere.
        for piece in bytes.chunks(chunk) {
            let _ = out.write_all(piece);
            let _ = out.flush();
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn send(&self, body: &Value) {
        let mut bytes = Self::frame(body);
        if self.options.flag("--glue") {
            // The answer and a notification in one write: two messages in one read.
            bytes.extend(Self::frame(&json!({
                "jsonrpc": "2.0", "method": "window/logMessage",
                "params": { "type": 4, "message": "glued" }
            })));
        }
        self.write(&bytes);
    }
}

fn capabilities(options: &Options) -> Value {
    let mut caps = serde_json::Map::new();
    match options.value("--encoding") {
        Some("utf-8") => caps.insert("positionEncoding".into(), json!("utf-8")),
        Some("utf-16") => caps.insert("positionEncoding".into(), json!("utf-16")),
        Some("utf-32") => caps.insert("positionEncoding".into(), json!("utf-32")),
        _ => None,
    };
    match options.value("--sync") {
        Some("full") => caps.insert("textDocumentSync".into(), json!(1)),
        Some("incremental") => caps.insert("textDocumentSync".into(), json!(2)),
        Some("object") => caps.insert(
            "textDocumentSync".into(),
            json!({ "openClose": true, "change": 2, "save": { "includeText": true } }),
        ),
        _ => None,
    };
    let legend =
        json!({ "tokenTypes": ["keyword", "function"], "tokenModifiers": ["declaration"] });
    match options.value("--semantic") {
        Some("full") => caps.insert(
            "semanticTokensProvider".into(),
            json!({ "legend": legend, "full": true }),
        ),
        Some("delta") => caps.insert(
            "semanticTokensProvider".into(),
            json!({ "legend": legend, "full": { "delta": true } }),
        ),
        Some("range") => caps.insert(
            "semanticTokensProvider".into(),
            json!({ "legend": legend, "range": true }),
        ),
        _ => None,
    };
    Value::Object(caps)
}

fn main() {
    let options = Options {
        args: std::env::args().skip(1).collect(),
    };
    let out = Output { options: &options };
    let log = |value: &Value| {
        if let Some(path) = options.value("--log")
            && let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path)
        {
            let _ = writeln!(file, "{value}");
        }
    };
    let mut input = std::io::BufReader::new(std::io::stdin().lock());
    let mut shutdown_seen = false;
    while let Some(message) = read_message(&mut input) {
        log(&message);
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let id = message.get("id").cloned();
        if options.value("--crash-on") == Some(method) && !method.is_empty() {
            std::process::exit(4);
        }
        match (method, id) {
            ("initialize", Some(id)) => {
                if options.flag("--init-error") {
                    out.send(&json!({
                        "jsonrpc": "2.0", "id": id,
                        "error": { "code": -32603, "message": "no usable workspace" }
                    }));
                    continue;
                }
                std::thread::sleep(Duration::from_millis(options.number("--init-delay-ms")));
                out.send(&json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": { "capabilities": capabilities(&options), "serverInfo": { "name": "fake" } }
                }));
            }
            ("initialized", None) => {
                if options.flag("--stderr-noise") {
                    eprintln!("fake server: starting up");
                    eprintln!("fake server: ready");
                }
                if options.flag("--ask-config") {
                    out.send(&json!({
                        "jsonrpc": "2.0", "id": 9001, "method": "workspace/configuration",
                        "params": { "items": [{ "section": "fake" }, { "section": "missing" }] }
                    }));
                }
                if options.flag("--ask-unknown") {
                    out.send(&json!({
                        "jsonrpc": "2.0", "id": 9002, "method": "client/registerCapability",
                        "params": { "registrations": [] }
                    }));
                }
                if options.flag("--log-message") {
                    out.send(&json!({
                        "jsonrpc": "2.0", "method": "window/logMessage",
                        "params": { "type": 3, "message": "hello from the fake server" }
                    }));
                }
                if options.flag("--garbage-after-init") {
                    let junk = b"this is not json";
                    let mut frame = format!("Content-Length: {}\r\n\r\n", junk.len()).into_bytes();
                    frame.extend_from_slice(junk);
                    out.write(&frame);
                }
                if options.flag("--bad-frame") {
                    out.write(b"garbage without a colon\r\n\r\n");
                }
                if options.flag("--oversize") {
                    out.write(b"Content-Length: 99999999\r\n\r\n");
                }
                if options.flag("--crash-after-init") {
                    std::process::exit(3);
                }
            }
            ("textDocument/didOpen" | "textDocument/didChange", None) => {
                // --diagnostics: one error over the first two characters, tagged
                // with the document version unless --diag-unversioned.
                if options.flag("--diagnostics") {
                    let document = &message["params"]["textDocument"];
                    let mut params = json!({
                        "uri": document["uri"],
                        "diagnostics": [{
                            "range": { "start": { "line": 0, "character": 0 },
                                       "end": { "line": 0, "character": 2 } },
                            "severity": 1, "message": "fake error", "source": "fake"
                        }]
                    });
                    if !options.flag("--diag-unversioned") {
                        params["version"] = document["version"].clone();
                    }
                    out.send(&json!({
                        "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": params
                    }));
                }
            }
            ("textDocument/didSave", None) => {
                if options.flag("--refresh-on-save") {
                    out.send(&json!({
                        "jsonrpc": "2.0", "id": 9100, "method": "workspace/semanticTokens/refresh"
                    }));
                }
            }
            ("textDocument/semanticTokens/full", Some(id)) => {
                std::thread::sleep(Duration::from_millis(options.number("--slow-ms")));
                // The first two characters are a keyword.
                let result = if options.flag("--semantic-null") {
                    Value::Null
                } else {
                    json!({ "data": [0, 0, 2, 0, 0] })
                };
                out.send(&json!({ "jsonrpc": "2.0", "id": id, "result": result }));
            }
            ("shutdown", Some(id)) => {
                shutdown_seen = true;
                if !options.flag("--no-shutdown-reply") {
                    out.send(&json!({ "jsonrpc": "2.0", "id": id, "result": null }));
                }
            }
            ("exit", None) => {
                if options.flag("--linger") {
                    // Ignore the exit request: the client has to end the process.
                    std::thread::sleep(Duration::from_secs(60));
                }
                std::process::exit(if shutdown_seen { 0 } else { 1 });
            }
            (_, Some(id)) if !method.is_empty() => {
                if options.flag("--ignore-requests") {
                    continue;
                }
                std::thread::sleep(Duration::from_millis(options.number("--slow-ms")));
                // Answer with what was asked, so tests see exactly what arrived.
                out.send(&json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": { "method": method, "params": message.get("params").cloned().unwrap_or(Value::Null) }
                }));
            }
            _ => {}
        }
    }
    // The client closed our input.
    std::process::exit(if shutdown_seen { 0 } else { 2 });
}
