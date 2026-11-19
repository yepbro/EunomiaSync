use std::{env, path::PathBuf};

fn main() {
    // The Windows manifest is a separate file (`windows/app.manifest`), so
    // edits to it must re-run this build script. tauri-build emits narrower
    // `rerun-if-changed` directives of its own (config files, capabilities,
    // resources) that do not cover it, and `include_str!` is opaque to
    // Cargo: without this directive Cargo would keep embedding a stale
    // manifest.
    println!("cargo:rerun-if-changed=windows/app.manifest");

    // How the manifest reaches Windows binaries (T-241). tauri-build's RC
    // embed travels through embed-resource's `cargo:rustc-link-arg-bins`,
    // which reaches ONLY `[[bin]]` targets: every cargo-test harness exe
    // (`eunomia_sync_lib-*.exe` and friends) links with no manifest at all,
    // binds comctl32 5.82, and dies at load with
    // STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139) because tao/tauri statically
    // import TaskDialogIndirect, a v6-only export (tauri-apps/tauri#13419).
    // On Windows+MSVC the linker therefore embeds the manifest for EVERY
    // linked target of this package and tauri-build's own embed is switched
    // off — the app exe cannot carry the RT_MANIFEST resource twice (the RC
    // embed and /MANIFEST:EMBED collide at link time). /MANIFESTINPUT takes
    // a file path, so unlike a RUSTFLAGS-borne /MANIFESTDEPENDENCY there is
    // no whitespace to re-quote on the cargo → rustc → link.exe way.
    let windows_msvc = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");

    let windows = if windows_msvc {
        let manifest =
            PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set"))
                .join("windows")
                .join("app.manifest");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT={}", manifest.display());
        tauri_build::WindowsAttributes::new_without_app_manifest()
    } else {
        // `WindowsAttributes::app_manifest` replaces the default manifest
        // wholesale (tauri-build 2.7.5 embeds only the Common-Controls v6
        // dependency by default), so `windows/app.manifest` reproduces that
        // dependency and adds the NFR-023 (longPathAware) and SEC-012/NFR-011
        // (asInvoker) settings. `config_tests.rs` pins all of it.
        tauri_build::WindowsAttributes::new().app_manifest(include_str!("windows/app.manifest"))
    };

    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("failed to run tauri-build");
}
