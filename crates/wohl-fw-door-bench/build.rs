//! Build script: hand `memory.x` to `cortex-m-rt`'s linker invocation.
//!
//! Only relevant when building for a bare-metal target. On host targets
//! (`cargo test`, `cargo clippy` without `--target`) we do nothing.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    // Only emit linker tweaks when targeting bare-metal.
    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("none") {
        return;
    }

    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR not set"));
    let memory_x = include_bytes!("memory.x");
    fs::write(out.join("memory.x"), memory_x).expect("could not write memory.x");

    println!("cargo:rustc-link-search={}", out.display());
    // Re-run the build script if memory.x is edited.
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=build.rs");
}
