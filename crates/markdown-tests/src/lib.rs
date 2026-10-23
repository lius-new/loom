//! Verification compiles the production Markdown files without GUI or I/O.
#![allow(dead_code)]
#[path = "../../../src/features/markdown/content.rs"]
pub mod content;
#[path = "../../../src/features/markdown/layout.rs"]
pub mod layout;
#[path = "../../../src/features/markdown/model.rs"]
pub mod model;
#[path = "../../../src/features/markdown/parser.rs"]
pub mod parser;
#[cfg(test)]
#[path = "../../../src/features/markdown/production_tests.rs"]
mod production_tests;
#[path = "../../../src/features/markdown/reader.rs"]
pub mod reader;
#[path = "../../../src/features/markdown/resources.rs"]
pub mod resources;
pub mod features {
    pub mod markdown {
        pub use crate::{content, layout, model, parser, reader, resources};
    }
}
