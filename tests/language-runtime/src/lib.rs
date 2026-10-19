//! External artifact integration suite against the production domain, parser
//! and worker sources. No grammar source or grammar crate dependency.
#![allow(dead_code)]
#![allow(unused_imports)]
#![cfg_attr(test, allow(linker_messages))]
#[path = "../../../src/domain/mod.rs"]
pub(crate) mod domain;
mod features;
mod infrastructure;

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
        serde_json::json!({"language":language.name(),"colored":frame[0].spans.iter().filter(|s|s.class.is_some()).count(),"modules":activity.modules,"errors":activity.errors})
    );
}
