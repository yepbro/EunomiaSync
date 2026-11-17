//! File-system layer for EunomiaSync.
//!
//! This crate owns every file-system interaction outside the database:
//! repository scanning, path normalization and validation, backup copies
//! before writes, and atomic (write-then-rename) file updates. It has no
//! dependency on Tauri so that file-system logic can be developed and
//! tested independently of the UI layer, and it keeps the "touch real
//! files" side effects out of the pure-logic `eunomia-core` and the
//! database-focused `eunomia-store` crates (see DECISIONS.md D-010).
//!
//! ## Module plan (docs/spec.md 5.1, 5.2)
//!
//! - [`scan`] — repository tree traversal
//! - [`paths`] — path normalization and validation
//! - [`backup`] — backup copies before writes
//! - [`atomic`] — atomic writes with retries
//!
//! The modules are declared but not implemented yet: this is the crate
//! skeleton, and the module boundaries above are its public contract.
//! Target-file watching (spec 5.7) is planned for this crate as well and
//! arrives with its own module and task, together with the
//! `notify-debouncer-full` dependency.

pub mod atomic;
pub mod backup;
pub mod paths;
pub mod scan;
