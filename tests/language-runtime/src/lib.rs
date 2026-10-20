//! External artifact integration suite against the production domain, parser
//! and worker sources. No grammar source or grammar crate dependency.
#![allow(dead_code)]
#![allow(unused_imports)]
#![cfg_attr(test, allow(linker_messages))]
#[path = "../../../src/domain/mod.rs"]
pub(crate) mod domain;
mod features;
mod infrastructure;

/// Validate producer artifacts using the real loader and query implementation.
pub fn validate_pack() -> Result<(), String> {
    use features::syntax::Resources;
    use std::path::PathBuf;
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(args.next().ok_or("missing package root")?);
    let samples = PathBuf::from(args.next().ok_or("missing validation samples")?);
    let resources = infrastructure::language_packs::Packages::discover(&root, None);
    let errors = resources.activity().errors;
    if !errors.is_empty() {
        return Err(errors.into_iter().collect::<Vec<_>>().join("\n"));
    }
    resources.registry()?;
    let samples: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(samples).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    for sample in samples {
        let grammar = sample["grammar"].as_str().ok_or("missing grammar")?;
        let file = sample["sample"].as_str().ok_or("missing sample path")?;
        let text = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
        let parser = resources.parser(grammar)?;
        let colored =
            features::syntax::validate_producer_grammar(&resources, grammar, parser, &text)?;
        println!(
            "{}",
            serde_json::json!({"grammar":grammar,"colored":colored})
        );
    }
    Ok(())
}

/// A headless production-mode probe: each invocation starts with a fresh
/// registry, using exactly the same installed package loader as Loom.
pub fn probe() {
    use features::syntax::{Source, Window};
    use std::{path::PathBuf, sync::atomic::AtomicBool};
    let file = PathBuf::from(std::env::args_os().nth(1).expect("input path"));
    infrastructure::language_packs::initialize();
    let language = domain::language::from_path(&file);
    let content = std::fs::read_to_string(&file).expect("input text");
    let text = loom_editor_core::text::TextBuffer::new(&content);
    let frame = infrastructure::language_packs::index()
        .prepare(
            domain::identity::FileId::new(1),
            Source::new(&text),
            language,
            &[Window {
                line: 0..text.len(),
                range: 0..text.len(),
            }],
            &AtomicBool::new(false),
        )
        .unwrap();
    let activity = infrastructure::language_packs::resources().activity();
    println!(
        "{}",
        serde_json::json!({"language":language.name(),"colored":frame[0].spans.iter().filter(|s|s.class.is_some()).count(),"modules":activity.modules,"wasm_modules":activity.wasm_modules,"errors":activity.errors})
    );
}
