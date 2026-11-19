//! Core library for EunomiaSync.
//!
//! Pure application logic with no input or output of its own: the
//! domain vocabulary of the app and the immutable build snapshot live
//! here, and the markdown, diff, sync, and watch logic will join them
//! in later tasks. This crate knows neither SQLite nor the file system
//! nor Tauri (spec §5.2; DECISIONS.md D-006, D-010): database access
//! lives in `eunomia-store`, file-system access in `eunomia-fs`, and
//! the Tauri command layer in `src-tauri`. The store layer assembles
//! [`snapshot::BuildSnapshot`] (spec §9.1, ALG-001) in one read
//! transaction and hands it over for building.
//!
//! The dependency surface is pinned by the spec (§5.2) and recorded in
//! DECISIONS.md D-017: `serde` (canonical forms of the domain
//! vocabulary), `thiserror` (domain error types) and `sha2` (idle
//! until the build-hash task). Nothing else joins without a
//! DECISIONS.md entry.
//!
//! ## Modules
//!
//! - [`project`] — the project write vocabulary: scheme, base name,
//!   extra files
//! - [`library`] — library items: instructions, headings, variants
//! - [`assignment`] — the project ↔ instruction ↔ variant link
//! - [`snapshot`] — the immutable build snapshot (ALG-001)
//! - [`normalize`] — the ALG-002 variant text normalization the build
//!   applies to every snapshot text before it reaches a file
//! - [`body`] — the ALG-020…022 assembly of a project's file body
//!   from a snapshot (§9.2)

pub mod assignment;
pub mod body;
pub mod extra_files;
pub mod library;
pub mod marker;
pub mod normalize;
pub mod project;
pub mod snapshot;
