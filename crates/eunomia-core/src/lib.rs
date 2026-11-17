//! Core library for EunomiaSync.
//!
//! This crate intentionally has no dependency on Tauri and no I/O of its
//! own: it holds the pure application logic — domain types, markdown,
//! diff, sync, and watch. Database access (SQLite storage and migrations)
//! lives in the `eunomia-store` crate (see DECISIONS.md D-010), and the
//! Tauri command layer lives in `src-tauri`.
