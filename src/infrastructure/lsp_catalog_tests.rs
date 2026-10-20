use super::*;
use crate::domain::language::Registry;
use std::path::Path;

const DIGEST: &str = "dc55afb7a43c70aebc75031186146de9cbb487eeddc0375143793696b6fb3695";

fn catalog(entry_extra: &str, platform_extra: &str) -> String {
    format!(
        r#"
schema = 1

[distribution]
repository = "lius-new/loom"

[servers.rust-analyzer]
name = "rust-analyzer"
version = "2026-10-12"
license = "MIT OR Apache-2.0"
command = "${{pack}}/rust-analyzer"
args = ["--stdio"]
priority = 5
root = {{ markers = ["Cargo.toml", "rust-project.json"], strategy = "outermost", fallback = "file_directory" }}
languages = [{{ language = "rust", language_id = "rust" }}]
{entry_extra}

[servers.rust-analyzer.platforms.windows-x86_64]
asset = "rust-analyzer-windows-x86_64.tar.gz"
release = "lsp-servers-2026.10.21.1"
sha256 = "{DIGEST}"
size = 12345678
{platform_extra}
"#
    )
}

fn error_of(text: &str) -> String {
    match Catalog::parse(text) {
        Ok(_) => panic!("accepted:\n{text}"),
        Err(error) => error,
    }
}

#[test]
fn a_catalog_entry_becomes_a_complete_server_definition() {
    let parsed = Catalog::parse(&catalog(
        "initialization_options = { checkOnSave = false }",
        "",
    ))
    .unwrap();
    let set = parsed.servers().unwrap();
    let server = set.get("rust-analyzer").expect("the entry's server");
    assert_eq!(&*server.display_name, "rust-analyzer");
    assert_eq!(&*server.command, "${pack}/rust-analyzer");
    assert_eq!(
        server.args.iter().map(|a| &**a).collect::<Vec<_>>(),
        ["--stdio"]
    );
    assert_eq!(server.priority, 5);
    assert_eq!(
        server.initialization_options.as_deref(),
        Some(r#"{"checkOnSave":false}"#)
    );
    let markers: Vec<_> = server.root.markers.iter().map(|m| &**m).collect();
    assert_eq!(markers, ["Cargo.toml", "rust-project.json"]);

    let registry = Registry::builtin();
    let rust = registry.resolve_path(Path::new("lib.rs"));
    let bound = set.for_language(registry, rust);
    assert_eq!(bound.len(), 1);
    assert_eq!((&*bound[0].0.name, bound[0].1), ("rust-analyzer", "rust"));
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
        assert!(
            !server.command.starts_with('/') || Path::new(&*server.command).is_absolute(),
            "{}",
            server.name
        );
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
fn bad_catalogs_are_rejected_with_a_reason() {
    let cases: Vec<(&str, String)> = vec![
        (
            "invalid language server catalog",
            catalog("", "").replace("schema = 1", "schema = 2"),
        ),
        (
            "invalid language server catalog",
            catalog("", "").replace("lius-new/loom", "lius-new/../loom"),
        ),
        (
            "unknown platform",
            catalog("", "").replace("windows-x86_64", "plan9-mips"),
        ),
        (
            "unexpected name",
            catalog("", "").replace("rust-analyzer-windows-x86_64.tar.gz", "ra.tar.gz"),
        ),
        (
            "release tag",
            catalog("", "").replace("lsp-servers-2026.10.21.1", "2026.10.21.1"),
        ),
        ("SHA-256", catalog("", "").replace(DIGEST, "abc")),
        (
            "size",
            catalog("", "").replace("size = 12345678", "size = 0"),
        ),
        (
            "size",
            catalog("", "").replace("size = 12345678", "size = 999999999999"),
        ),
        (
            "not a server name",
            catalog("", "").replace("[servers.rust-analyzer", "[servers.Rust_Analyzer"),
        ),
        (
            "name, version and license",
            catalog("", "").replace("license = \"MIT OR Apache-2.0\"", "license = \"\""),
        ),
        (
            "no platform",
            catalog("", "").replace(
                "[servers.rust-analyzer.platforms.windows-x86_64]",
                "[servers.rust-analyzer.unused]",
            ),
        ),
    ];
    for (expected, text) in cases {
        // Malformed shape fails to parse; bad values fail validation.
        let error = error_of(&text);
        assert!(
            error.contains(expected)
                || expected == "no platform"
                || expected == "not a server name",
            "expected {expected:?} in {error:?}"
        );
    }
    assert!(Catalog::parse(&catalog("surprise = 1", "")).is_err());
    assert!(Catalog::parse("not toml {").is_err());
}

#[test]
fn a_definition_the_language_service_rejects_fails_the_catalog() {
    let catalog = Catalog::parse(&catalog("", "").replace(
        r#"languages = [{ language = "rust", language_id = "rust" }]"#,
        r#"languages = [{ language = "Rust", language_id = "rust" }]"#,
    ))
    .unwrap();
    assert!(catalog.servers().unwrap_err().contains("canonical"));
}
