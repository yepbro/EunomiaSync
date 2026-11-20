//! Database layer for EunomiaSync.
//!
//! This crate owns everything SQLite: the schema migrations and the
//! storage/repository code on top of them. It intentionally has no
//! dependency on Tauri so that database logic can be developed and
//! tested independently of the UI layer (see DECISIONS.md D-010).

pub mod action_log;
pub mod projects;
pub mod settings;
pub mod storage;
pub mod values;
