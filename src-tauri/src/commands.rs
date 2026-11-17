//! Tauri commands exposed to the frontend.
//!
//! Every command is annotated with `#[specta::specta]` so that
//! `tauri-specta` can include it in the generated TypeScript bindings
//! (`src/bindings.ts`). Do not call `invoke()` directly from the
//! frontend — always use the generated `commands`.

/// Greets a user by name. Template demo command, kept as the
/// reference example for adding real commands.
#[tauri::command]
#[specta::specta]
pub fn greet(name: String) -> String {
    format!("Hello, {name}! You've been greeted from Rust!")
}
