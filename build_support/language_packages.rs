//! Stage independent package manifests and queries; native modules are built by
//! their own workspace crates and copied by the release packagers.
use std::{collections::BTreeSet, fs, path::Path};

pub fn stage(root: &Path, destination: &Path) {
    let sources = root.join("assets/language-packs");
    let mut current = BTreeSet::new();
    for entry in fs::read_dir(sources).expect("language package directory") {
        let entry = entry.expect("language package entry");
        let directory = entry.path();
        let manifest = fs::read(directory.join("package.json")).expect("language package manifest");
        let value: serde_json::Value = serde_json::from_slice(&manifest).expect("package JSON");
        let id = value["id"].as_str().expect("package id");
        assert_eq!(directory.file_name().unwrap(), id);
        current.insert(id.to_owned());
        let target = destination.join(id);
        fs::create_dir_all(&target).expect("package directory");
        write_changed(&target.join("package.json"), &manifest);
        for grammar in value["grammars"]
            .as_object()
            .expect("package grammars")
            .keys()
        {
            let source = root.join("assets/grammars").join(grammar);
            let target = target.join("grammars").join(grammar);
            fs::create_dir_all(&target).expect("grammar directory");
            for file in ["highlights.scm", "SOURCE.md", "injections.scm"] {
                let source = source.join(file);
                let target = target.join(file);
                if file == "injections.scm" && !source.is_file() {
                    if target.is_file() {
                        fs::remove_file(target).expect("remove stale optional query");
                    }
                    continue;
                }
                write_changed(&target, &fs::read(source).expect("grammar resource"));
            }
        }
    }
    // A removed/renamed source package must stop being discoverable in an
    // incremental build. Leave its payload files alone: a running editor may
    // still have the previous package snapshot and native module handles.
    if let Ok(entries) = fs::read_dir(destination) {
        for entry in entries {
            let entry = entry.expect("staged package entry");
            if entry.file_type().is_ok_and(|kind| kind.is_dir())
                && !current.contains(entry.file_name().to_string_lossy().as_ref())
            {
                let manifest = entry.path().join("package.json");
                if manifest.is_file() {
                    fs::remove_file(manifest).expect("remove stale generated package manifest");
                }
            }
        }
    }
}

fn write_changed(path: &Path, bytes: &[u8]) {
    if fs::read(path).ok().as_deref() != Some(bytes) {
        fs::write(path, bytes).expect("stage language package resource");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incremental_staging_keeps_packages_independent_and_removes_stale_registration() {
        struct Temp(std::path::PathBuf);
        impl Drop for Temp {
            fn drop(&mut self) {
                assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let temp =
            Temp(std::env::temp_dir().join(format!("loom-stage-packages-{}", std::process::id())));
        let root = temp.0.join("source");
        let target = temp.0.join("output");
        let packages = root.join("assets/language-packs");
        let queries = root.join("assets/grammars/sample");
        fs::create_dir_all(&queries).unwrap();
        for file in ["highlights.scm", "SOURCE.md", "injections.scm"] {
            fs::write(queries.join(file), file).unwrap();
        }
        let manifest = |id: &str| {
            let dir = packages.join(id);
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("package.json"),
                serde_json::json!({
                    "schema":1,"id":id,"languages":[],
                    "grammars":{"sample":{"module":"sample","symbol":"tree_sitter_sample"}}
                })
                .to_string(),
            )
            .unwrap();
        };
        manifest("first");
        stage(&root, &target);
        assert!(target.join("first/package.json").is_file());
        assert!(
            target
                .join("first/grammars/sample/injections.scm")
                .is_file()
        );
        fs::remove_file(queries.join("injections.scm")).unwrap();
        stage(&root, &target);
        assert!(!target.join("first/grammars/sample/injections.scm").exists());
        // Rename a source package and build again using the same output folder.
        fs::rename(packages.join("first"), packages.join("second")).unwrap();
        manifest("second");
        stage(&root, &target);
        assert!(!target.join("first/package.json").exists());
        assert!(
            target
                .join("first/grammars/sample/highlights.scm")
                .is_file()
        );
        assert!(target.join("second/package.json").is_file());
        assert!(!target.join("catalog.json").exists());
    }
}
