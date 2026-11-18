fn main() {
    // The Windows manifest is a separate file (`windows/app.manifest`), so
    // edits to it must re-run this build script. tauri-build emits narrower
    // `rerun-if-changed` directives of its own (config files, capabilities,
    // resources) that do not cover it, and `include_str!` is opaque to
    // Cargo: without this directive Cargo would keep embedding a stale
    // manifest.
    println!("cargo:rerun-if-changed=windows/app.manifest");

    // `WindowsAttributes::app_manifest` replaces the default manifest
    // wholesale (tauri-build 2.7.5 embeds only the Common-Controls v6
    // dependency by default), so `windows/app.manifest` reproduces that
    // dependency and adds the NFR-023 (longPathAware) and SEC-012/NFR-011
    // (asInvoker) settings. `config_tests.rs` pins all of it.
    let windows =
        tauri_build::WindowsAttributes::new().app_manifest(include_str!("windows/app.manifest"));

    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("failed to run tauri-build");
}
