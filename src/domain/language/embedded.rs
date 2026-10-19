//! Production contains only the plain-text fallback descriptor. Tests use the
//! bundled metadata fixtures. Grammar/query payloads always stay on disk.
use crate::domain::language::registry::{Registry, RegistryError};

include!(concat!(env!("OUT_DIR"), "/language_assets.rs"));

/// The embedded tables, for tests that compare them with the files on disk.
pub(crate) fn files() -> (&'static [Row], &'static [Row], &'static [Row]) {
    (LANGUAGE_FILES, GRAMMAR_FILES, SERVER_FILES)
}

/// (name, file, path from the repository root, contents)
pub(crate) type Row = (&'static str, &'static str, &'static str, &'static str);

/// The registry for the embedded descriptions and grammar directories.
pub(crate) fn registry() -> Result<Registry, RegistryError> {
    let sources: Vec<(&str, &str)> = LANGUAGE_FILES
        .iter()
        .map(|(name, _, _, contents)| (*name, *contents))
        .collect();
    let servers: Vec<(&str, &str)> = SERVER_FILES
        .iter()
        .map(|(name, _, _, contents)| (*name, *contents))
        .collect();
    Registry::from_parts(&sources, GRAMMAR_NAMES, &servers)
}

#[cfg(test)]
mod tests {
    use crate::domain::language::descriptor::{RootFallback as Fallback, RootStrategy as Strategy};
    use crate::domain::language::embedded::*;
    use std::path::Path;

    #[test]
    fn the_embedded_registry_builds_and_keeps_the_previous_recognition() {
        let registry = Registry::builtin();
        let expect = |path: &str, name: &str, variant: &str| {
            let resolved = registry.resolve_path(Path::new(path));
            assert_eq!(&*registry.definition_of(resolved).name, name, "{path}");
            assert_eq!(registry.variant_name(resolved), variant, "{path}");
        };
        // The extension table of the former Language::from_path.
        expect("main.RS", "rust", "rust");
        expect("a.cs", "csharp", "csharp");
        for ts in ["a.ts", "a.mts", "a.cts"] {
            expect(ts, "typescript", "typescript");
        }
        expect("component.tsx", "typescript", "tsx");
        for js in ["a.js", "a.jsx", "script.mjs", "a.cjs"] {
            expect(js, "javascript", "javascript");
        }
        expect("README.md", "markdown", "markdown");
        expect("notes.txt", "plaintext", "plaintext");
        expect("Makefile", "plaintext", "plaintext");
    }

    #[test]
    fn display_data_moved_out_of_code_matches_what_the_status_bar_showed() {
        let registry = Registry::builtin();
        let shown = |name: &str| {
            let definition = registry.definition(registry.language_by_name(name).unwrap());
            (
                definition.badge.to_string(),
                definition.display_name.to_string(),
            )
        };
        assert_eq!(shown("csharp"), ("C#".into(), "C#".into()));
        assert_eq!(shown("rust"), ("RS".into(), "Rust".into()));
        assert_eq!(shown("typescript"), ("TS".into(), "TypeScript".into()));
        assert_eq!(shown("javascript"), ("JS".into(), "JavaScript".into()));
        assert_eq!(shown("plaintext"), ("TXT".into(), "Plain Text".into()));
        assert_eq!(
            registry.language_by_name("ts"),
            registry.language_by_name("typescript")
        );
    }
    #[test]
    fn rust_is_served_by_rust_analyzer_and_its_root_policy_is_data() {
        let registry = Registry::builtin();
        let rust = registry.default_resolved(registry.language_by_name("rust").unwrap());
        let (server, language_id) = registry.server_for(rust).expect("rust has a server");
        assert_eq!((&*server.name, language_id), ("rust-analyzer", "rust"));
        assert_eq!(&*server.command, "rust-analyzer");
        let markers: Vec<_> = server.root.markers.iter().map(|m| &**m).collect();
        assert_eq!(markers, ["Cargo.toml", "rust-project.json"]);
        assert_eq!(server.root.strategy, Strategy::Outermost);
        assert_eq!(server.root.fallback, Fallback::FileDirectory);
        for other in ["plaintext", "csharp", "javascript", "typescript"] {
            let resolved = registry.default_resolved(registry.language_by_name(other).unwrap());
            assert!(registry.server_for(resolved).is_none(), "{other}");
        }
    }
}
