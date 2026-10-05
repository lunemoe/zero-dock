//! Link configuration for the Zero Dock Rust crate.
//!
//! The crate is linked as a cdylib (the panel plugin module) and as an rlib
//! (the Rust test-host binary). Both consume symbols from libxfce4panel and
//! libxfce4windowing, which ship no pkg-config based Rust bindings, so the
//! link flags are emitted here via pkg-config.

fn main() {
    for pkg in ["libxfce4panel-2.0", "libxfce4windowing-0", "xcomposite"] {
        match pkg_config::probe_library(pkg) {
            Ok(lib) => {
                for path in &lib.link_paths {
                    println!("cargo:rustc-link-search=native={}", path.display());
                }
                for name in &lib.libs {
                    println!("cargo:rustc-link-lib=dylib={}", name);
                }
            }
            Err(err) => {
                println!("cargo:warning=pkg-config lookup for {pkg} failed: {err}");
            }
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}
