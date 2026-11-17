//! API test for the `greet` command, routed through a real IPC round trip
//! against the Tauri mock runtime (no windowing system involved).
//!
//! Lives inside the lib crate because `tauri::generate_handler!` requires
//! the command's generated helper macros, which are crate-private.
//! The `InvokeRequest` shape mirrors tauri's own `tauri::test` doctest for
//! the installed version; update it together with the `tauri` dependency.

use tauri::test::{mock_builder, mock_context, noop_assets};

#[test]
fn greet_responds_over_mock_ipc() {
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![crate::commands::greet])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");

    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("failed to create test webview window");

    let request = tauri::webview::InvokeRequest {
        cmd: "greet".into(),
        callback: tauri::ipc::CallbackFn(0),
        error: tauri::ipc::CallbackFn(1),
        url: if cfg!(any(windows, target_os = "android")) {
            "http://tauri.localhost".parse().unwrap()
        } else {
            "tauri://localhost".parse().unwrap()
        },
        body: tauri::ipc::InvokeBody::Json(serde_json::json!({ "name": "Eunomia" })),
        headers: Default::default(),
        invoke_key: tauri::test::INVOKE_KEY.to_string(),
    };

    let response = tauri::test::get_ipc_response(&webview, request)
        .expect("greet command should respond successfully");
    let greeting: String = response
        .deserialize()
        .expect("greet response should be a JSON string");

    assert_eq!(greeting, "Hello, Eunomia! You've been greeted from Rust!");
}

#[test]
fn greet_rejects_unknown_command_payload() {
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![crate::commands::greet])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");

    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("failed to create test webview window");

    // The command argument is `name`; anything else must be rejected.
    let request = tauri::webview::InvokeRequest {
        cmd: "greet".into(),
        callback: tauri::ipc::CallbackFn(0),
        error: tauri::ipc::CallbackFn(1),
        url: if cfg!(any(windows, target_os = "android")) {
            "http://tauri.localhost".parse().unwrap()
        } else {
            "tauri://localhost".parse().unwrap()
        },
        body: tauri::ipc::InvokeBody::Json(serde_json::json!({ "wrong": "payload" })),
        headers: Default::default(),
        invoke_key: tauri::test::INVOKE_KEY.to_string(),
    };

    let outcome = tauri::test::get_ipc_response(&webview, request);
    assert!(
        outcome.is_err(),
        "a payload without `name` must be rejected"
    );
}
