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

    // The manifest above covers only the app exe. Cargo builds this
    // crate's TEST binaries without any embedded manifest, so the
    // Windows loader binds them to Common-Controls v5 — whose export
    // table lacks the v6-only entry points — and every eunomia-sync
    // lib-test exe dies at startup with STATUS_ENTRYPOINT_NOT_FOUND
    // (0xc0000139). Embedding the same v6 dependency into the test
    // binaries gives them an activation context that restores the v6
    // binding. Gated on the target OS (the build script itself runs on
    // the host, which is Linux in local checkouts).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-tests=/MANIFESTDEPENDENCY:type='win32' \
             name='Microsoft.Windows.Common-Controls' version='6.0.0.0' \
             publicKeyToken='6595b64144ccf1df' language='*' \
             processorArchitecture='*'"
        );
    }

    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("failed to run tauri-build");
}
