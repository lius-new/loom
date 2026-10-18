//! Finds the built-in language descriptions and grammar resources and renders
//! the static table that embeds them. It only uses `std`, is shared by
//! `build.rs`, and is also compiled into the test binary so its checks run with
//! `cargo test`.
//!
//! Layout it accepts, and nothing else:
//!
//! ```text
//! assets/languages/<name>.json
//! assets/servers/<name>.json              (hyphens allowed in the name)
//! assets/grammars/README.md                 (optional, not embedded)
//! assets/grammars/<grammar>/highlights.scm  (required)
//! assets/grammars/<grammar>/SOURCE.md       (required)
//! ```
//!
//! Names are lowercase ASCII (`rust`, `c_sharp`). Anything unexpected fails the
//! scan, so a misplaced or misspelled file cannot silently go missing.
use std::fs;
use std::path::{Path, PathBuf};

pub const LANGUAGES_DIR: &str = "assets/languages";
pub const GRAMMARS_DIR: &str = "assets/grammars";
pub const SERVERS_DIR: &str = "assets/servers";
const GRAMMAR_FILES: [&str; 2] = ["highlights.scm", "SOURCE.md"];
const GRAMMARS_NOTE: &str = "README.md";

#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    /// Language or grammar name.
    pub name: String,
    /// File name inside a grammar directory; the descriptor's file name for a language.
    pub file: String,
    /// Forward-slash path from the repository root.
    pub relative: String,
    pub absolute: PathBuf,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Scan {
    pub languages: Vec<Entry>,
    pub grammars: Vec<Entry>,
    pub servers: Vec<Entry>,
}

pub fn is_canonical_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// A language server name: lowercase words that may be joined by hyphens, as
/// server programs usually are named (`rust-analyzer`).
pub fn is_server_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Sorted directory entries as `(file name, is real file, is real directory)`.
/// A symbolic link is neither, so it is reported as unexpected by the caller.
fn list(dir: &Path) -> Result<Vec<(String, bool, bool)>, String> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|name| format!("{}: non-UTF-8 name {name:?}", dir.display()))?;
        let kind = fs::symlink_metadata(entry.path())
            .map_err(|e| format!("{}: {e}", entry.path().display()))?
            .file_type();
        entries.push((name, kind.is_file(), kind.is_dir()));
    }
    entries.sort();
    Ok(entries)
}

pub fn scan(root: &Path) -> Result<Scan, String> {
    let mut result = Scan::default();
    let languages = root.join(LANGUAGES_DIR);
    for (file, is_file, _) in list(&languages)? {
        let Some(name) = file.strip_suffix(".json").filter(|_| is_file) else {
            return Err(format!(
                "{LANGUAGES_DIR}/{file}: only <name>.json description files are allowed"
            ));
        };
        if !is_canonical_name(name) {
            return Err(format!(
                "{LANGUAGES_DIR}/{file}: `{name}` is not a canonical lowercase name"
            ));
        }
        result.languages.push(Entry {
            name: name.to_owned(),
            absolute: languages.join(&file),
            relative: format!("{LANGUAGES_DIR}/{file}"),
            file,
        });
    }

    let servers = root.join(SERVERS_DIR);
    for (file, is_file, _) in list(&servers)? {
        let Some(name) = file.strip_suffix(".json").filter(|_| is_file) else {
            return Err(format!(
                "{SERVERS_DIR}/{file}: only <name>.json server definitions are allowed"
            ));
        };
        if !is_server_name(name) {
            return Err(format!(
                "{SERVERS_DIR}/{file}: `{name}` is not a lowercase server name"
            ));
        }
        result.servers.push(Entry {
            name: name.to_owned(),
            absolute: servers.join(&file),
            relative: format!("{SERVERS_DIR}/{file}"),
            file,
        });
    }

    let grammars = root.join(GRAMMARS_DIR);
    for (name, is_file, is_dir) in list(&grammars)? {
        if is_file && name == GRAMMARS_NOTE {
            continue;
        }
        if !is_dir || !is_canonical_name(&name) {
            return Err(format!(
                "{GRAMMARS_DIR}/{name}: expected a grammar directory with a canonical lowercase name"
            ));
        }
        let dir = grammars.join(&name);
        let mut found = Vec::new();
        for (file, is_file, _) in list(&dir)? {
            if !is_file || !GRAMMAR_FILES.contains(&file.as_str()) {
                return Err(format!(
                    "{GRAMMARS_DIR}/{name}/{file}: only {GRAMMAR_FILES:?} are allowed"
                ));
            }
            found.push(file);
        }
        for required in GRAMMAR_FILES {
            if !found.iter().any(|file| file == required) {
                return Err(format!("{GRAMMARS_DIR}/{name}: missing {required}"));
            }
        }
        for file in found {
            result.grammars.push(Entry {
                name: name.clone(),
                absolute: dir.join(&file),
                relative: format!("{GRAMMARS_DIR}/{name}/{file}"),
                file,
            });
        }
    }
    Ok(result)
}

/// The whole build step: validate the resources and write the embedded table to
/// `OUT_DIR`. The table is rewritten from the current scan every time, so a
/// removed or renamed resource cannot leave a stale entry behind. Paths in the
/// `rerun-if-changed` lines are absolute because the calling package may not be
/// the repository root.
pub fn emit_build_step(root: &Path) {
    println!(
        "cargo:rerun-if-changed={}",
        root.join(LANGUAGES_DIR).display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        root.join(GRAMMARS_DIR).display()
    );
    let scan =
        scan(root).unwrap_or_else(|error| panic!("invalid built-in language assets: {error}"));
    let output = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"))
        .join("language_assets.rs");
    let rendered = render(&scan);
    if fs::read_to_string(&output).ok().as_deref() != Some(rendered.as_str()) {
        fs::write(&output, rendered).expect("write the embedded language asset table");
    }
}

/// Rust source for the embedded tables. The order follows the scan, which is
/// sorted, so the same resources always give the same table.
pub fn render(scan: &Scan) -> String {
    fn table(entries: &[Entry]) -> String {
        let mut out = String::new();
        for entry in entries {
            // `{:?}` on a str is a valid Rust string literal, backslashes included.
            out.push_str(&format!(
                "    ({:?}, {:?}, {:?}, include_str!({:?})),\n",
                entry.name,
                entry.file,
                entry.relative,
                entry.absolute.to_string_lossy()
            ));
        }
        out
    }
    format!(
        "/// (name, file, path from the repository root, contents)\n\
         pub(crate) static LANGUAGE_FILES: &[(&str, &str, &str, &str)] = &[\n{}];\n\
         /// (grammar, file, path from the repository root, contents)\n\
         pub(crate) static GRAMMAR_FILES: &[(&str, &str, &str, &str)] = &[\n{}];\n\
         /// (server, file, path from the repository root, contents)\n\
         pub(crate) static SERVER_FILES: &[(&str, &str, &str, &str)] = &[\n{}];\n",
        table(&scan.languages),
        table(&scan.grammars),
        table(&scan.servers)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A label, the files to create, and the text the error must contain.
    type Case<'a> = (&'a str, &'a [(&'a str, &'a str)], &'a str);
    type Owned = (String, String, String, String);

    struct Tree(PathBuf);
    impl Tree {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "loom-language-assets-{label}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join(LANGUAGES_DIR)).unwrap();
            fs::create_dir_all(root.join(GRAMMARS_DIR)).unwrap();
            fs::create_dir_all(root.join(SERVERS_DIR)).unwrap();
            Self(root)
        }
        fn file(&self, relative: &str, contents: &str) -> &Self {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
            self
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn error(tree: &Tree) -> String {
        scan(&tree.0).unwrap_err()
    }

    #[test]
    fn accepts_the_documented_layout_in_sorted_order() {
        let tree = Tree::new("ok");
        tree.file("assets/languages/zeta.json", "{}")
            .file("assets/languages/alpha.json", "{}")
            .file("assets/grammars/README.md", "note")
            .file("assets/grammars/c_sharp/highlights.scm", "")
            .file("assets/grammars/c_sharp/SOURCE.md", "")
            .file("assets/grammars/ada/SOURCE.md", "")
            .file("assets/grammars/ada/highlights.scm", "")
            .file("assets/servers/b-server.json", "{}")
            .file("assets/servers/a_server.json", "{}");
        let scan = scan(&tree.0).unwrap();
        let names = |entries: &[Entry]| {
            entries
                .iter()
                .map(|e| format!("{}:{}", e.name, e.file))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(&scan.languages),
            ["alpha:alpha.json", "zeta:zeta.json"]
        );
        assert_eq!(
            names(&scan.grammars),
            [
                "ada:SOURCE.md",
                "ada:highlights.scm",
                "c_sharp:SOURCE.md",
                "c_sharp:highlights.scm"
            ]
        );
        assert_eq!(scan.grammars[0].relative, "assets/grammars/ada/SOURCE.md");
        assert!(scan.grammars.iter().all(|e| e.name != "README"));
    }

    #[test]
    fn rejects_anything_unexpected() {
        let cases: [Case; 10] = [
            (
                "stray language file",
                &[("assets/languages/notes.txt", "")],
                "only <name>.json",
            ),
            (
                "upper case name",
                &[("assets/languages/Rust.json", "")],
                "canonical",
            ),
            (
                "dash in name",
                &[("assets/languages/c-sharp.json", "")],
                "canonical",
            ),
            (
                "nested language directory",
                &[("assets/languages/extra/a.json", "")],
                "only <name>.json",
            ),
            (
                "stray server file",
                &[("assets/servers/notes.txt", "")],
                "only <name>.json",
            ),
            (
                "upper case server",
                &[("assets/servers/Rust-Analyzer.json", "")],
                "lowercase server name",
            ),
            (
                "grammar missing its source note",
                &[("assets/grammars/rust/highlights.scm", "")],
                "missing SOURCE.md",
            ),
            (
                "unknown grammar file",
                &[
                    ("assets/grammars/rust/highlights.scm", ""),
                    ("assets/grammars/rust/SOURCE.md", ""),
                    ("assets/grammars/rust/injections.scm", ""),
                ],
                "injections.scm",
            ),
            (
                "nested grammar directory",
                &[
                    ("assets/grammars/rust/highlights.scm", ""),
                    ("assets/grammars/rust/SOURCE.md", ""),
                    ("assets/grammars/rust/queries/a.scm", ""),
                ],
                "queries",
            ),
            (
                "loose grammar file",
                &[("assets/grammars/notes.txt", "")],
                "grammar directory",
            ),
        ];
        for (label, files, expected) in cases {
            let tree = Tree::new(&label.replace(' ', "-"));
            for (path, contents) in files {
                tree.file(path, contents);
            }
            let message = error(&tree);
            assert!(message.contains(expected), "{label}: {message}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symbolic_links() {
        let tree = Tree::new("links");
        tree.file("target.json", "{}");
        std::os::unix::fs::symlink(
            tree.0.join("target.json"),
            tree.0.join("assets/languages/linked.json"),
        )
        .unwrap();
        assert!(error(&tree).contains("only <name>.json"));
    }

    #[test]
    fn scanning_twice_gives_identical_output_and_valid_literals() {
        let tree = Tree::new("render");
        tree.file("assets/languages/b.json", "{}")
            .file("assets/languages/a.json", "{}");
        let first = render(&scan(&tree.0).unwrap());
        assert_eq!(first, render(&scan(&tree.0).unwrap()));
        assert!(first.find("\"a\"").unwrap() < first.find("\"b\"").unwrap());
        assert!(first.contains("include_str!("));
        assert!(first.contains("LANGUAGE_FILES") && first.contains("GRAMMAR_FILES"));
        // Backslashes in a Windows path must be escaped to stay a valid literal.
        let entry = Entry {
            name: "x".into(),
            file: "x.json".into(),
            relative: "assets/languages/x.json".into(),
            absolute: PathBuf::from(r"C:\a b\x.json"),
        };
        let rendered = render(&Scan {
            languages: vec![entry],
            grammars: Vec::new(),
            servers: Vec::new(),
        });
        assert!(rendered.contains(r#"include_str!("C:\\a b\\x.json")"#));
    }

    /// The compiled table must be exactly the files in the repository. This
    /// lists the disk on its own, so the generated table is checked against the
    /// files and not against itself.
    #[test]
    fn the_embedded_table_is_exactly_what_is_on_disk() {
        use crate::domain::language::{Row, embedded_files};
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let scanned = scan(root).unwrap();
        // Contents are compared with line endings normalised: a checkout may
        // have converted them.
        let disk = |entries: &[Entry]| -> Vec<Owned> {
            entries
                .iter()
                .map(|e| {
                    let text = fs::read_to_string(&e.absolute).unwrap();
                    let text = text.replace("\r\n", "\n");
                    (e.name.clone(), e.file.clone(), e.relative.clone(), text)
                })
                .collect()
        };
        let embedded = |rows: &[Row]| -> Vec<Owned> {
            rows.iter()
                .map(|(a, b, c, d)| {
                    let text = d.replace("\r\n", "\n");
                    (a.to_string(), b.to_string(), c.to_string(), text)
                })
                .collect()
        };
        let (languages, grammars, servers) = embedded_files();
        assert_eq!(embedded(languages), disk(&scanned.languages));
        assert_eq!(embedded(grammars), disk(&scanned.grammars));
        assert_eq!(embedded(servers), disk(&scanned.servers));
        // Names that differ only by case would collide on case-insensitive disks.
        let mut lowered: Vec<_> = languages
            .iter()
            .map(|(n, ..)| n.to_ascii_lowercase())
            .collect();
        lowered.sort();
        lowered.dedup();
        assert_eq!(lowered.len(), languages.len());
    }

    #[test]
    fn canonical_names_are_lowercase_ascii_words() {
        for good in ["rust", "c_sharp", "tsx", "a1"] {
            assert!(is_canonical_name(good), "{good}");
        }
        for bad in ["", "Rust", "1a", "c-sharp", "c sharp", "中文", "_a"] {
            assert!(!is_canonical_name(bad), "{bad}");
        }
    }
}
