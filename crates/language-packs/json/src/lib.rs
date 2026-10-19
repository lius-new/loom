#![allow(linker_messages)]

#[unsafe(no_mangle)]
pub extern "C" fn loom_tree_sitter_json() -> *const () {
    // SAFETY: the generated grammar exports a valid Tree-sitter C function.
    unsafe { tree_sitter_json::LANGUAGE.into_raw()() }
}
