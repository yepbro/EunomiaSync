//! Core library for EunomiaSync.
//!
//! This crate intentionally has no dependency on Tauri so that the
//! application logic (storage, markdown, diff, sync, watch) can be
//! developed and tested independently of the UI layer.
//! See `src-tauri` for the Tauri command layer.

pub mod storage;
