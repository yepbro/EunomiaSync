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
//! The module boundaries above are the crate's public contract
//! (DECISIONS.md D-019): implementing tasks fit their code into the
//! declared modules. [`paths`] carries the implemented pieces so far —
//! the project-folder availability check (spec 6.9, 9.4), the
//! project-path canonicalization with the `path_key` identity key
//! (DB-030, DB-031, D-07), the Git repository root lookup (PRJ-003),
//! and the sync target-path resolution with its symlink checks (9.4
//! preflight step 2, 9.5 step 1); `scan`, `backup` and `atomic` are
//! still
//! skeletons. Target-file watching
//! (spec 5.7) is planned for this crate as well and arrives with its own
//! module and task, together with the
//! `notify-debouncer-full` dependency.

pub mod atomic;
pub mod backup;
pub mod free_space;
pub mod paths;
pub mod scan;
pub mod temp_files;
pub mod write_access;
