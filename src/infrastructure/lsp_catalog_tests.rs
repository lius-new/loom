use super::*;
use crate::domain::language::Registry;
use std::path::Path;

const DIGEST: &str = "dc55afb7a43c70aebc75031186146de9cbb487eeddc0375143793696b6fb3695";
/// What the publication pipeline generates for its production recipes; its own
/// tests keep this file in step with the recipes.
const GOLDEN: &str = include_str!("../../scripts/lsp-servers/fixtures/catalog.toml");

fn catalog(server_extra: &str, pack_extra: &str) -> String {
    format!(
        r#"
schema = 1

[distribution]
repository = "lius-new/loom"

[packs.node]
version = "22.23.3"
license = "MIT"

[packs.node.platforms.windows-x86_64]
asset = "node-windows-x86_64.tar.gz"
release = "lsp-servers-2026.10.21.1"
sha256 = "{DIGEST}"
size = 35000000

[packs.pyright]
version = "1.1.414"
license = "MIT"
requires = ["node@22"]
{pack_extra}

[packs.pyright.platforms.any]
asset = "pyright-any.tar.gz"
release = "lsp-servers-2026.10.21.1"
sha256 = "{DIGEST}"
size = 12345678

[servers.pyright]
name = "Pyright"
pack = "pyright"
command = "${{runtime.node}}/node"
args = ["${{pack}}/node_modules/pyright/langserver.index.js", "--stdio"]
priority = 10
{server_extra}

[servers.pyright.root]
markers = ["pyrightconfig.json", "pyproject.toml"]
strategy = "nearest"
fallback = "workspace"

[[servers.pyright.languages]]
language = "python"
language_id = "python"
"#
    )
}

fn error_of(text: &str) -> String {
    match Catalog::parse(text) {
        Ok(_) => panic!("accepted:\n{text}"),
        Err(error) => error,
    }
}

fn python() -> Registry {
    let python = r#"{"name":"python","display_name":"Python","badge":"PY","variants":[{"id":"python","grammar":"python"}],"files":[{"extensions":["py"],"variant":"python"}]}"#;
    let plain = r#"{"name":"plaintext","display_name":"Plain","badge":"T","variants":[{"id":"plaintext"}]}"#;
    Registry::from_sources(&[("python", python), ("plaintext", plain)], &["python"]).unwrap()
}

#[test]
fn a_catalog_entry_becomes_a_server_definition_that_starts_from_its_packs() {
    let parsed = Catalog::parse(&catalog("initialization_options = { key = false }", "")).unwrap();
    let set = parsed.servers().unwrap();
    let server = set.get("pyright").expect("the entry's server");
    assert_eq!(&*server.display_name, "Pyright");
    assert_eq!(&*server.command, "${runtime.node}/node");
    assert_eq!(
        server.args.iter().map(|a| &**a).collect::<Vec<_>>(),
        [
            "${pack}/node_modules/pyright/langserver.index.js",
            "--stdio"
        ]
    );
    assert_eq!(server.priority, 10);
    assert_eq!(server.pack.as_deref(), Some("pyright"));
    assert_eq!(
        server.initialization_options.as_deref(),
        Some(r#"{"key":false}"#)
    );
    let registry = python();
    let file = registry.resolve_path(Path::new("main.py"));
    let bound = set.for_language(&registry, file);
    assert_eq!(bound.len(), 1);
    assert_eq!((&*bound[0].0.name, bound[0].1), ("pyright", "python"));
}

#[test]
fn the_catalog_the_pipeline_generates_is_readable_and_orders_servers_for_a_language() {
    let parsed = Catalog::parse(GOLDEN).expect("the pipeline's catalog format");
    let set = parsed.servers().expect("every generated server is valid");
    assert_eq!(set.all().len(), 17);

    // Several servers for one language come out in priority order, each with the
    // language id it expects.
    let registry = Registry::builtin();
    let bound = |path: &str| -> Vec<(String, String)> {
        let resolved = registry.resolve_path(Path::new(path));
        set.for_language(registry, resolved)
            .into_iter()
            .map(|(server, id)| (server.name.to_string(), id.to_owned()))
            .collect()
    };
    assert_eq!(bound("lib.rs"), [("rust-analyzer".into(), "rust".into())]);
    assert_eq!(
        bound("a.ts"),
        [
            ("typescript-language-server".into(), "typescript".into()),
            ("biome".into(), "typescript".into()),
            ("vscode-eslint-language-server".into(), "typescript".into()),
        ]
    );
    assert_eq!(
        bound("a.tsx"),
        [
            (
                "typescript-language-server".into(),
                "typescriptreact".into()
            ),
            ("biome".into(), "typescriptreact".into()),
            (
                "vscode-eslint-language-server".into(),
                "typescriptreact".into()
            ),
        ]
    );
    assert_eq!(
        bound("a.css")
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["biome", "vscode-css-language-server"]
    );
    assert_eq!(bound("README.md"), [("marksman".into(), "markdown".into())]);

    // Every generated command starts from a pack, never from PATH.
    for server in set.all() {
        assert!(
            server.command.starts_with("${pack}/") || server.command.starts_with("${runtime."),
            "{}",
            server.name
        );
    }
    let registry = python();
    let resolved = registry.resolve_path(Path::new("main.py"));
    let python: Vec<_> = set
        .for_language(&registry, resolved)
        .into_iter()
        .map(|(server, _)| server.name.to_string())
        .collect();
    assert_eq!(python, ["pyright", "ruff"]);
}

#[test]
fn a_catalog_without_servers_is_valid_and_defines_none() {
    let parsed =
        Catalog::parse("schema = 1\n[distribution]\nrepository = \"lius-new/loom\"\n").unwrap();
    assert!(parsed.servers().unwrap().all().is_empty());
}

#[test]
fn the_shipped_catalog_is_valid() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/lsp-servers/lsp-servers.toml");
    let set = load(&file).expect("the catalog that ships with Loom");
    for server in set.all() {
        assert!(server.command.starts_with("${"), "{}", server.name);
    }
}

#[test]
fn a_missing_file_means_no_servers() {
    let set = load(&std::env::temp_dir().join("loom-no-such-lsp-catalog.toml")).unwrap();
    assert!(set.all().is_empty());
}

#[test]
fn the_catalog_is_staged_beside_the_executable() {
    #[cfg(not(target_os = "macos"))]
    assert_eq!(
        catalog_file(Path::new("/app/Loom.exe")),
        Path::new("/app/lsp-servers/lsp-servers.toml")
    );
    #[cfg(target_os = "macos")]
    assert_eq!(
        catalog_file(Path::new("/Loom.app/Contents/MacOS/Loom")),
        Path::new("/Loom.app/Contents/Resources/lsp-servers/lsp-servers.toml")
    );
}

#[test]
fn a_server_never_starts_a_program_found_on_path() {
    let base = catalog("", "");
    let cases = [
        (
            "command must start with",
            base.replace("command = \"${runtime.node}/node\"", "command = \"node\""),
        ),
        (
            "command must start with",
            base.replace(
                "command = \"${runtime.node}/node\"",
                "command = \"/usr/bin/node\"",
            ),
        ),
        (
            "command must start with",
            base.replace("${runtime.node}", "${runtime.python}"),
        ),
        (
            "command must start with",
            base.replace("requires = [\"node@22\"]", "requires = []"),
        ),
    ];
    for (expected, text) in cases {
        let error = error_of(&text);
        assert!(error.contains(expected), "expected {expected:?}: {error}");
    }
    // A pack's own program is fine.
    let own = base.replace("${runtime.node}/node", "${pack}/pyright");
    assert!(Catalog::parse(&own).is_ok());
}

#[test]
fn bad_catalogs_are_rejected_with_a_reason() {
    let base = catalog("", "");
    let cases: Vec<(&str, String)> = vec![
        ("invalid language server catalog", base.replace("schema = 1", "schema = 2")),
        (
            "invalid language server catalog",
            base.replace("lius-new/loom", "lius-new/../loom"),
        ),
        (
            "unknown platform",
            base.replace("platforms.windows-x86_64", "platforms.plan9-mips"),
        ),
        (
            "unexpected name",
            base.replace("node-windows-x86_64.tar.gz", "node.tar.gz"),
        ),
        (
            "release tag",
            base.replace("lsp-servers-2026.10.21.1", "2026.10.21.1"),
        ),
        ("SHA-256", base.replace(DIGEST, "abc")),
        ("size", base.replace("size = 12345678", "size = 0")),
        ("size", base.replace("size = 12345678", "size = 999999999999")),
        (
            "not a server name",
            base.replace("[servers.pyright", "[servers.Py_Right"),
        ),
        (
            "version and license",
            base.replace("license = \"MIT\"\nrequires", "license = \"\"\nrequires"),
        ),
        (
            "not provided by the catalog",
            base.replace("node@22", "node@18"),
        ),
        (
            "not in the catalog",
            base.replace("pack = \"pyright\"", "pack = \"missing\""),
        ),
        (
            "either the platform any or specific platforms",
            base.replace(
                "[packs.pyright.platforms.any]",
                "[packs.pyright.platforms.linux-x86_64]\nasset = \"pyright-linux-x86_64.tar.gz\"\nrelease = \"lsp-servers-1\"\nsha256 = \"0000000000000000000000000000000000000000000000000000000000000000\"\nsize = 1\n\n[packs.pyright.platforms.any]",
            ),
        ),
    ];
    for (expected, text) in cases {
        let error = error_of(&text);
        assert!(
            error.contains(expected),
            "expected {expected:?} in {error:?}"
        );
    }
    assert!(Catalog::parse(&catalog("surprise = 1", "")).is_err());
    assert!(Catalog::parse("not toml {").is_err());
}

#[test]
fn a_definition_the_language_service_rejects_fails_the_catalog() {
    let parsed = Catalog::parse(&catalog("", "").replace(
        "language = \"python\"\nlanguage_id = \"python\"",
        "language = \"Python\"\nlanguage_id = \"python\"",
    ))
    .unwrap();
    assert!(parsed.servers().unwrap_err().contains("canonical"));
}
