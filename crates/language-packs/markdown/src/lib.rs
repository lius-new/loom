#![allow(linker_messages)]

#[unsafe(no_mangle)]
pub extern "C" fn loom_tree_sitter_markdown() -> *const () {
    // SAFETY: the generated grammar exports a valid Tree-sitter C function.
    unsafe { tree_sitter_md::LANGUAGE.into_raw()() }
}

#[unsafe(no_mangle)]
pub extern "C" fn loom_tree_sitter_markdown_inline() -> *const () {
    // SAFETY: the generated grammar exports a valid Tree-sitter C function.
    unsafe { tree_sitter_md::INLINE_LANGUAGE.into_raw()() }
}
