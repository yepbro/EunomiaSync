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
//! ## Modules (docs/spec.md 5.1, 5.2)
//!
//! - [`scan`] — repository tree traversal (spec 9.6, with
//!   [`scan_classify`] as its ALG-052/053 classification half)
//! - [`paths`] — path normalization, validation and target-path
//!   resolution
//! - [`backup`] — backup copies before writes (§7.6)
//! - [`atomic`] — atomic writes with retries (§9.5)
//! - [`free_space`], [`temp_files`], [`write_access`] — the §9.4
//!   preflight seams (volume space, the staging-name grammar and its
//!   sweep, write rights)
//! - `fault_injection` — spec 15.1's failure points for tests,
//!   behind the non-default `fault-injection` feature (absent from
//!   release builds by construction; see its docs)
//! - `temp_repos` — self-cleaning temporary `.git` repository roots
//!   for the §12.2 benchmark data sets (fixture tooling, not product
//!   flow; task T-136)
//!
//! The module boundaries above are the crate's public contract
//! (DECISIONS.md D-019): implementing tasks fit their code into the
//! declared modules. Every module listed is implemented —
//! [`paths`] since the availability/canonicalization/root-lookup
//! tasks, [`scan`]/[`scan_classify`] since T-061/T-068,
//! [`free_space`]/[`temp_files`]/[`write_access`] since the §9.4
//! preflight tasks, [`backup`] since T-064/D-091 and [`atomic`]
//! since T-066/T-067 (D-095/D-099). Target-file watching
//! (spec 5.7) is still planned for this crate: it arrives with its
//! own module and task, together with the
//! `notify-debouncer-full` dependency.

pub mod atomic;
pub mod backup;
#[cfg(any(test, feature = "fault-injection"))]
pub mod fault_injection;
pub mod free_space;
pub mod paths;
pub mod scan;
pub mod scan_classify;
pub mod temp_files;
pub mod temp_repos;
pub mod write_access;
