//! Host-only loader checks use synthetic metadata and never require a grammar.
use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};

static RUN: AtomicUsize = AtomicUsize::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "loom-host-packs-{}-{}",
            std::process::id(),
            RUN.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn write(&self, id: &str, value: &Value) {
        let dir = self.0.join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("package.json"), value.to_string()).unwrap();
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn manifest() -> Value {
    json!({"schema":1,"id":"sample","languages":[{
        "name":"sample","display_name":"Sample","badge":"S",
        "variants":[{"id":"sample","grammar":"sample"}],
        "files":[{"extensions":["sample"],"variant":"sample"}]
    }],"grammars":{"sample":{"module":"missing_module","symbol":"tree_sitter_sample"}}})
}

#[test]
fn standard_entry_points_are_inferred_and_explicit_aliases_are_preserved() {
    let default: Module = serde_json::from_value(json!({"module":"python.dll"})).unwrap();
    assert_eq!(default.entry_point("python"), "tree_sitter_python");
    let alias: Module =
        serde_json::from_value(json!({"module":"python.dll","symbol":"tree_sitter_python"}))
            .unwrap();
    assert_eq!(alias.entry_point("my_python"), "tree_sitter_python");
    let temp = Temp::new();
    let mut value = manifest();
    value["grammars"]["sample"] = json!({"module":"sample.dll"});
    temp.write("sample", &value);
    let source = Packages::discover(&temp.0, None);
    assert!(
        source
            .registry()
            .unwrap()
            .language_by_name("sample")
            .is_some()
    );
    assert_eq!(source.activity(), Activity::default());
}

#[test]
fn standard_native_filenames_are_used_without_renaming_or_path_escape() {
    for (platform, extension) in [("windows", "dll"), ("linux", "so"), ("macos", "dylib")] {
        assert_eq!(
            module_file("python", platform).unwrap(),
            format!("python.{extension}")
        );
        for file in [
            "python.dll",
            "python.so",
            "python.dylib",
            "libtree-sitter-python.so",
        ] {
            assert_eq!(module_file(file, platform).unwrap(), file);
        }
    }
    for invalid in [
        "../python.dll",
        "a/python.so",
        "a\\python.dll",
        "C:python.dll",
        "python..so",
        "python.so.exe",
        "python.json",
        "",
    ] {
        assert!(module_file(invalid, "windows").is_err(), "{invalid}");
    }
}

#[test]
fn no_packages_provide_plain_text_and_no_resource_activity() {
    let temp = Temp::new();
    let source = Packages::discover(&temp.0.join("missing"), None);
    let registry = source.registry().unwrap();
    assert_eq!(registry.languages().len(), 1);
    for file in ["a.md", "a.json", "a.rs"] {
        assert_eq!(
            &*registry
                .definition_of(registry.resolve_path(Path::new(file)))
                .name,
            "plaintext"
        );
    }
    assert_eq!(source.activity(), Activity::default());
}

#[test]
fn metadata_recognition_is_lazy_and_missing_payload_failure_is_cached() {
    let temp = Temp::new();
    temp.write("sample", &manifest());
    let source = Packages::discover(&temp.0, None);
    let registry = source.registry().unwrap();
    assert_eq!(
        &*registry
            .definition_of(registry.resolve_path(Path::new("a.sample")))
            .name,
        "sample"
    );
    assert_eq!(source.activity(), Activity::default());
    assert!(source.language("sample").is_err());
    let failed = source.activity();
    assert_eq!(failed.language_loads, 1);
    assert!(source.language("sample").is_err());
    assert_eq!(source.activity(), failed);
    assert!(source.text("sample", "highlights.scm").is_err());
    let failed = source.activity();
    assert!(source.text("sample", "highlights.scm").is_err());
    assert_eq!(source.activity(), failed);
}

#[test]
fn overrides_disable_and_restart_are_metadata_only() {
    let app = Temp::new();
    let user = Temp::new();
    app.write("sample", &manifest());
    let first = Packages::discover(&app.0, Some(&user.0));
    let mut replacement = manifest();
    replacement["enabled"] = json!(false);
    user.write("sample", &replacement);
    assert!(
        first
            .registry()
            .unwrap()
            .language_by_name("sample")
            .is_some()
    );
    let next = Packages::discover(&app.0, Some(&user.0));
    assert!(
        next.registry()
            .unwrap()
            .language_by_name("sample")
            .is_none()
    );
    assert_eq!(next.activity(), Activity::default());
    replacement["schema"] = json!(99);
    user.write("sample", &replacement);
    let invalid = Packages::discover(&app.0, Some(&user.0));
    assert!(
        invalid
            .registry()
            .unwrap()
            .language_by_name("sample")
            .is_some()
    );
    assert_eq!(invalid.activity().errors.len(), 1);
}

#[test]
fn invalid_packages_do_not_corrupt_valid_metadata() {
    let temp = Temp::new();
    temp.write("sample", &manifest());
    for (id, value) in [
        (
            "path",
            json!({"schema":1,"id":"path","languages":[],"grammars":{"../escape":{"module":"x","symbol":"x"}}}),
        ),
        (
            "field",
            json!({"schema":1,"id":"field","languages":[],"grammars":{},"unknown":true}),
        ),
        (
            "zconflict",
            json!({"schema":1,"id":"zconflict","languages":[],"grammars":{"sample":{"module":"x","symbol":"x"}}}),
        ),
    ] {
        temp.write(id, &value);
    }
    // A conflicting incoming package is rejected atomically.
    let source = Packages::discover(&temp.0, None);
    assert!(
        source
            .registry()
            .unwrap()
            .language_by_name("sample")
            .is_some()
    );
    assert_eq!(source.activity().errors.len(), 3);
    assert_eq!(source.activity().language_loads, 0);
}

#[test]
fn platform_paths_are_absolute_and_portable_directories_are_not_cargo_profiles() {
    let home = std::env::temp_dir().join("loom-data");
    assert_eq!(
        user_directory_from("windows", |_| Some(home.clone().into_os_string())),
        Some(home.join("Loom/language-packs"))
    );
    assert_eq!(
        user_directory_from("windows", |_| Some("relative".into())),
        None
    );
    assert_eq!(
        locations(Path::new("/portable/deps/Loom.exe")),
        PathBuf::from("/portable/deps/language-packs")
    );
}
