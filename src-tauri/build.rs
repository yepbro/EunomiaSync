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
    //
    // T-241: this RC embed only reaches `[[bin]]` targets (embed-resource
    // links it through `cargo:rustc-link-arg-bins`), so cargo-test harness
    // exes link with no manifest, bind comctl32 5.82, and die at load with
    // STATUS_ENTRYPOINT_NOT_FOUND (tauri-apps/tauri#13419). Delivering the
    // manifest to test exes from HERE turned out unreliable on cargo 1.100
    // (T-241 iteration 1: /MANIFEST:EMBED + /MANIFESTINPUT via plain
    // `cargo:rustc-link-arg` compiled fine but changed nothing at runtime),
    // so CI passes the same file to the linker for every test exe through a
    // space-free RUSTFLAGS instead (see ci.yml) — RUSTFLAGS cannot carry
    // the wordy /MANIFESTDEPENDENCY form, but /MANIFESTINPUT=<path> has no
    // spaces and survives cargo's word splitting verbatim.
    let windows =
        tauri_build::WindowsAttributes::new().app_manifest(include_str!("windows/app.manifest"));

    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("failed to run tauri-build");
}
