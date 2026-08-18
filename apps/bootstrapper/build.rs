//! Embeds the application manifest.
//!
//! Not a code generator and not a downloader — it emits two linker arguments and
//! nothing else. `implicit-build-downloads = false` is a workspace policy and
//! this build script is inventoried in `dependency-policy/build-scripts.json`
//! alongside every other one in the graph.
//!
//! Done through the MSVC linker rather than a manifest crate on purpose: it
//! needs no new dependency, and this workspace pins dependencies exactly and
//! reviews each one. See the manifest itself for what it declares and for the
//! measurements that made it necessary.

use std::path::Path;

fn main() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("speakeasy-bootstrapper.manifest");
    println!("cargo:rerun-if-changed={}", manifest.display());

    // MSVC only. The product is Windows-only, but the workspace still has to
    // parse on any host a developer runs `cargo metadata` on, and a GNU or
    // non-Windows link would reject these flags outright.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // `-bins`, not the crate-wide form: applying manifest embedding to every
        // link unit would also apply it to test binaries, where it is noise.
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
            manifest.display()
        );
        // Without this the linker merges in its own default manifest fragment
        // requesting `asInvoker` as well, and duplicate `trustInfo` elements are
        // a hard manifest-tool error rather than a warning.
        println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:NO");
    }
}
