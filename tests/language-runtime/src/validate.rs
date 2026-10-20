#![allow(linker_messages)]
fn main() {
    if let Err(error) = loom_language_runtime_tests::validate_pack() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
