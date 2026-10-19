// Tree-sitter/Wasmtime's C API is statically linked on MSVC; the linker resolves
// the upstream headers' dllimport annotations within that same executable.
#![allow(linker_messages)]
fn main() {
    loom_language_runtime_tests::probe();
}
