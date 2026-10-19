// Discover independent language packages at startup; open native modules and
// queries only when the syntax worker requests them. Changes require restart.
use crate::domain::language::Registry;
use crate::features::syntax::{Resources, TreeIndex};
use libloading::Library;
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};
use tree_sitter::Language;
use tree_sitter_language::LanguageFn;

const MAX_MANIFEST: u64 = 1024 * 1024;
const MAX_QUERY: u64 = 4 * 1024 * 1024;
const MAX_QUERY_CACHE: usize = 8 * 1024 * 1024;
const MAX_MODULES: usize = 64;
const MAX_PACKAGES: usize = 128;
const PLAIN: &str = r#"{"name":"plaintext","display_name":"Plain Text","badge":"TXT","variants":[{"id":"plaintext"}]}"#;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Module {
    module: String,
    #[serde(default)]
    symbol: Option<String>,
}
impl Module {
    fn entry_point(&self, grammar: &str) -> String {
        self.symbol
            .clone()
            .unwrap_or_else(|| format!("tree_sitter_{grammar}"))
    }
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    id: String,
    #[serde(default = "enabled")]
    enabled: bool,
    languages: Vec<Value>,
    #[serde(default)]
    servers: Vec<Value>,
    grammars: BTreeMap<String, Module>,
}
fn enabled() -> bool {
    true
}

fn name(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}
// Accept an unmodified shared library filename (including Neovim's .so files
// on macOS), or a stem whose platform extension is added without a lib prefix.
fn module_file(module: &str, platform: &str) -> Result<String, String> {
    if name(module) {
        let extension = match platform {
            "windows" => "dll",
            "macos" => "dylib",
            _ => "so",
        };
        return Ok(format!("{module}.{extension}"));
    }
    if let Some((stem, extension)) = module.rsplit_once('.') {
        let mut chars = stem.chars();
        if chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
            && matches!(extension, "dll" | "so" | "dylib")
        {
            return Ok(module.to_owned());
        }
    }
    Err(format!("invalid native module filename {module}"))
}
fn read_text(path: &Path, limit: u64) -> Result<String, String> {
    let file = fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut text = String::new();
    file.take(limit + 1)
        .read_to_string(&mut text)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if text.len() as u64 > limit {
        return Err(format!("{} exceeds {limit} bytes", path.display()));
    }
    Ok(text)
}

#[derive(Clone)]
struct Installed {
    manifest: Manifest,
    root: PathBuf,
}

fn registry(packages: &BTreeMap<String, Installed>) -> Result<Registry, String> {
    let mut languages = BTreeMap::from([("plaintext".to_owned(), PLAIN.to_owned())]);
    let mut servers = BTreeMap::new();
    let mut grammars = BTreeSet::new();
    for package in packages.values() {
        for grammar in package.manifest.grammars.keys() {
            if !grammars.insert(grammar.as_str()) {
                return Err(format!("grammar {grammar} is defined by multiple packages"));
            }
        }
        for (values, target) in [
            (&package.manifest.languages, &mut languages),
            (&package.manifest.servers, &mut servers),
        ] {
            for value in values {
                let key = value
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "package descriptor has no name".to_owned())?;
                if target.insert(key.to_owned(), value.to_string()).is_some() {
                    return Err(format!("descriptor {key} is defined by multiple packages"));
                }
            }
        }
    }
    if grammars.len() > 128 || languages.len() > 129 || servers.len() > 64 {
        return Err("too many installed language descriptors".into());
    }
    let sources: Vec<_> = languages
        .iter()
        .map(|(n, j)| (n.as_str(), j.as_str()))
        .collect();
    let servers: Vec<_> = servers
        .iter()
        .map(|(n, j)| (n.as_str(), j.as_str()))
        .collect();
    let grammars: Vec<_> = grammars.into_iter().collect();
    Registry::from_parts(&sources, &grammars, &servers).map_err(|e| e.to_string())
}

impl Manifest {
    fn validate(&self, directory: &Path) -> Result<(), String> {
        if self.schema != 1 {
            return Err(format!(
                "unsupported language package schema {}",
                self.schema
            ));
        }
        if !name(&self.id) || directory.file_name().is_none_or(|n| n != self.id.as_str()) {
            return Err("package directory must match its canonical id".into());
        }
        for (grammar, module) in &self.grammars {
            if !name(grammar)
                || module_file(&module.module, std::env::consts::OS).is_err()
                || module.symbol.as_deref().is_some_and(|symbol| !name(symbol))
            {
                return Err(format!("invalid language module entry {grammar}"));
            }
        }
        // Each package is self-contained; references cannot depend on scan order.
        registry(&BTreeMap::from([(
            self.id.clone(),
            Installed {
                manifest: self.clone(),
                root: directory.to_owned(),
            },
        )]))
        .map(|_| ())
    }
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub(crate) struct Activity {
    pub language_loads: usize,
    pub query_reads: usize,
    pub modules: BTreeSet<String>,
    pub queries: BTreeSet<(String, String)>,
    pub errors: BTreeSet<String>,
}
type TextResult = Result<Option<Arc<str>>, String>;
#[derive(Default)]
struct Loaded {
    languages: HashMap<String, Result<Language, String>>,
    texts: HashMap<(String, String), TextResult>,
    text_bytes: usize,
    activity: Activity,
}

pub(crate) struct Packages {
    packages: BTreeMap<String, Installed>,
    loaded: Mutex<Loaded>,
}
impl Packages {
    pub(crate) fn discover(bundled: &Path, user: Option<&Path>) -> Self {
        let mut source = Self {
            packages: BTreeMap::new(),
            loaded: Mutex::default(),
        };
        for root in [Some(bundled), user].into_iter().flatten() {
            source.scan(root);
        }
        source
    }

    fn scan(&mut self, root: &Path) {
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                Self::report(
                    self.loaded.get_mut().unwrap(),
                    &format!("{}: {e}", root.display()),
                );
                return;
            }
        };
        let mut directories = Vec::new();
        for entry in entries {
            match entry {
                Ok(entry) => {
                    if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                        directories.push(entry.path());
                        if directories.len() > MAX_PACKAGES {
                            Self::report(
                                self.loaded.get_mut().unwrap(),
                                &format!("{}: too many language packages", root.display()),
                            );
                            return;
                        }
                    }
                }
                Err(e) => Self::report(
                    self.loaded.get_mut().unwrap(),
                    &format!("{}: {e}", root.display()),
                ),
            }
        }
        directories.sort();
        for directory in directories {
            // Ignore unrelated folders and partially downloaded packages.
            if !directory.join("package.json").is_file() {
                continue;
            }
            let result = (|| {
                let path = directory.join("package.json");
                let manifest: Manifest = serde_json::from_str(&read_text(&path, MAX_MANIFEST)?)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                manifest.validate(&directory)?;
                let mut candidate = self.packages.clone();
                if manifest.enabled {
                    candidate.insert(
                        manifest.id.clone(),
                        Installed {
                            manifest,
                            root: directory.clone(),
                        },
                    );
                } else {
                    candidate.remove(&manifest.id);
                }
                if candidate.len() > MAX_PACKAGES {
                    return Err("too many installed language packages".into());
                }
                registry(&candidate)?;
                self.packages = candidate;
                Ok::<_, String>(())
            })();
            if let Err(error) = result {
                Self::report(
                    self.loaded.get_mut().unwrap(),
                    &format!("{}: {error}", directory.display()),
                );
            }
        }
    }

    pub(crate) fn registry(&self) -> Result<Registry, String> {
        registry(&self.packages)
    }
    pub(crate) fn activity(&self) -> Activity {
        self.loaded.lock().unwrap().activity.clone()
    }
    pub(crate) fn grammar_names(&self) -> Vec<String> {
        self.packages
            .values()
            .flat_map(|p| p.manifest.grammars.keys().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    fn grammar(&self, grammar: &str) -> Option<(&Installed, &Module)> {
        self.packages
            .values()
            .find_map(|p| p.manifest.grammars.get(grammar).map(|m| (p, m)))
    }
    fn module_path(package: &Installed, module: &str) -> PathBuf {
        // Validated during discovery; never search Cargo or another package.
        package
            .root
            .join("modules")
            .join(module_file(module, std::env::consts::OS).expect("validated module filename"))
    }
    fn report(loaded: &mut Loaded, error: &str) {
        if loaded.activity.errors.insert(error.to_owned()) {
            eprintln!("language package: {error}");
        }
    }
}
// Tree-sitter's Language, Tree and Query all contain pointers into native
// modules. Keep handles until process exit, including after a worker is dropped.
// Catalog/package updates take effect on restart; this cache has a fixed cap.
fn load_native(path: &Path, symbol: &str) -> Result<Language, String> {
    static MODULES: OnceLock<Mutex<HashMap<PathBuf, Library>>> = OnceLock::new();
    let mut modules = MODULES.get_or_init(Mutex::default).lock().unwrap();
    let key = path
        .canonicalize()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if !modules.contains_key(&key) {
        if modules.len() >= MAX_MODULES {
            return Err("native language module limit reached; restart to release modules".into());
        }
        // SAFETY: packages are installed application resources, loaded through
        // an explicit absolute path, with an agreed Tree-sitter C entry point.
        let library =
            unsafe { Library::new(&key) }.map_err(|e| format!("{}: {e}", path.display()))?;
        modules.insert(key.clone(), library);
    }
    let library = &modules[&key];
    // SAFETY: standard Tree-sitter entry points use this C signature. Keep the
    // owning Library in MODULES for the entire lifetime of every Language.
    let function = unsafe { library.get::<unsafe extern "C" fn() -> *const ()>(symbol.as_bytes()) }
        .map_err(|e| format!("{} ({symbol}): {e}", path.display()))?;
    // Reject an invalid null entry point before Tree-sitter dereferences it.
    if unsafe { function() }.is_null() {
        return Err(format!(
            "{} ({symbol}) returned a null grammar",
            path.display()
        ));
    }
    let language = Language::new(unsafe { LanguageFn::from_raw(*function) });
    let version = language.abi_version();
    if !(tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION)
        .contains(&version)
    {
        return Err(format!(
            "{} uses unsupported Tree-sitter ABI {version}",
            path.display()
        ));
    }
    Ok(language)
}

impl Resources for Packages {
    fn language(&self, grammar: &str) -> Result<Language, String> {
        let mut loaded = self.loaded.lock().unwrap();
        if let Some(language) = loaded.languages.get(grammar) {
            return language.clone();
        }
        loaded.activity.language_loads += 1;
        let result = match self.grammar(grammar) {
            Some((package, module)) => {
                loaded.activity.modules.insert(module.module.clone());
                load_native(
                    &Self::module_path(package, &module.module),
                    &module.entry_point(grammar),
                )
            }
            None => Err(format!("language package {grammar} is not installed")),
        };
        if let Err(error) = &result {
            Self::report(&mut loaded, error);
        }
        loaded.languages.insert(grammar.to_owned(), result.clone());
        result
    }
    fn text(&self, grammar: &str, file: &str) -> TextResult {
        let Some((package, _)) = self.grammar(grammar) else {
            return Err(format!("unknown language resource {grammar}/{file}"));
        };
        if !["highlights.scm", "injections.scm", "SOURCE.md"].contains(&file) {
            return Err(format!("unknown language resource {grammar}/{file}"));
        }
        let mut loaded = self.loaded.lock().unwrap();
        let key = (grammar.to_owned(), file.to_owned());
        if let Some(text) = loaded.texts.get(&key) {
            return text.clone();
        }
        loaded.activity.queries.insert(key.clone());
        loaded.activity.query_reads += 1;
        let path = package.root.join("grammars").join(grammar).join(file);
        let result = if file == "injections.scm" && !path.is_file() {
            Ok(None)
        } else {
            read_text(&path, MAX_QUERY).and_then(|text| {
                if loaded.text_bytes + text.len() > MAX_QUERY_CACHE {
                    return Err("language query cache budget exceeded".to_owned());
                }
                loaded.text_bytes += text.len();
                Ok(Some(Arc::from(text)))
            })
        };
        if let Err(error) = &result {
            Self::report(&mut loaded, error);
        }
        loaded.texts.insert(key, result.clone());
        result
    }
    fn report(&self, error: &str) {
        Self::report(&mut self.loaded.lock().unwrap(), error);
    }
}
fn locations(executable: &Path) -> PathBuf {
    let parent = executable.parent().expect("executable has a directory");
    if cfg!(target_os = "macos")
        && parent.file_name().is_some_and(|name| name == "MacOS")
        && parent
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "Contents")
    {
        parent.parent().unwrap().join("Resources/language-packs")
    } else {
        parent.join("language-packs")
    }
}

fn user_directory_from(
    platform: &str,
    get: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    let home = || get("HOME").map(PathBuf::from);
    let base = match platform {
        "windows" => get("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("Loom")),
        "macos" => home().map(|p| p.join("Library/Application Support/Loom")),
        _ => get("XDG_DATA_HOME")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| home().map(|p| p.join(".local/share")))
            .map(|p| p.join("loom")),
    };
    base.filter(|p| p.is_absolute())
        .map(|p| p.join("language-packs"))
}
fn user_directory() -> Option<PathBuf> {
    user_directory_from(std::env::consts::OS, |name| std::env::var_os(name))
}

pub(crate) fn resources() -> Arc<Packages> {
    static RESOURCES: OnceLock<Arc<Packages>> = OnceLock::new();
    RESOURCES
        .get_or_init(|| {
            #[cfg(not(test))]
            let executable = std::env::current_exe().expect("locate Loom language resources");
            #[cfg(test)]
            {
                let root = std::env::var_os("LOOM_TEST_LANGUAGE_PACKS");
                let source = match root {
                    Some(path) => Packages::discover(Path::new(&path), None),
                    None => Packages {
                        packages: BTreeMap::new(),
                        loaded: Mutex::default(),
                    },
                };
                Arc::new(source)
            }
            #[cfg(not(test))]
            {
                Arc::new(Packages::discover(
                    &locations(&executable),
                    user_directory().as_deref(),
                ))
            }
        })
        .clone()
}
pub(crate) fn initialize() {
    let source = resources();
    if let Ok(registry) = source.registry() {
        if let Err(error) = Registry::install(registry) {
            eprintln!("language packages: {error}");
        }
    }
}
pub(crate) fn index() -> TreeIndex {
    TreeIndex::new(resources())
}

#[cfg(test)]
#[path = "language_packs_unit_tests.rs"]
mod unit_tests;
