//! The domain sources embed the built-in language assets, so this package needs
//! the same generated table the application gets.
#[path = "../../build_support/language_assets.rs"]
mod language_assets;

fn main() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .and_then(std::path::Path::parent)
        .expect("the package lives two levels below the repository root");
    language_assets::emit_build_step(root);
}
