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
    // manifest to test exes from the build script or RUSTFLAGS turned out
    // to be a dead end on every front (T-241 iterations 1-3: build-script
    // link args changed nothing at runtime on cargo 1.100; RUSTFLAGS
    // /MANIFEST:EMBED + /MANIFESTINPUT collides with this very RC embed on
    // the bin test harness — CVT1100 duplicate resource; /MANIFESTINPUT
    // without embed is refused — LNK1220). CI instead stages this file as
    // an external <exe>.manifest next to every test exe: the loader picks
    // it up for manifest-less exes and ignores it where the embedded one
    // exists. See the staging step in ci.yml.
    let windows =
        tauri_build::WindowsAttributes::new().app_manifest(include_str!("windows/app.manifest"));

    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("failed to run tauri-build");
}
