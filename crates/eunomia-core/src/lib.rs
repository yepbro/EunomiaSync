//! Core library for EunomiaSync.
//!
//! Pure application logic with no application input or output of its
//! own: the domain vocabulary of the app and the immutable build
//! snapshot live here, and the markdown, diff, sync, and watch logic
//! will join them in later tasks. This crate knows neither SQLite nor
//! the file system nor Tauri (spec §5.2; DECISIONS.md D-006, D-010):
//! database access lives in `eunomia-store`, file-system access in
//! `eunomia-fs`, and the Tauri command layer in `src-tauri`. The store
//! layer assembles [`snapshot::BuildSnapshot`] (spec §9.1, ALG-001)
//! in one read transaction and hands it over for building. One
//! environment read exists by decision (D-104, an exemption to §5.2's
//! no-I/O rule recorded there): the creation-marker clock in
//! [`marker_clock`] reads the process clock and the system time zone
//! database — environment inputs, not application data, and confined
//! to that module's single capture function.
//!
//! The dependency surface is pinned by the spec (§5.2) and recorded in
//! DECISIONS.md D-017: `serde` (canonical forms of the domain
//! vocabulary), `thiserror` (domain error types) and `sha2` (the
//! build hash, D-064, and the color tie-break, D-077). `jiff` joined
//! for the creation-marker clock (ALG-032's once-per-run capture,
//! D-104). Nothing else joins without a DECISIONS.md entry.
//!
//! ## Modules
//!
//! - [`project`] — the project write vocabulary: scheme, base name,
//!   extra files, palette color
//! - [`library`] — library items: instructions, headings, variants
//! - [`assignment`] — the project ↔ instruction ↔ variant link
//! - [`snapshot`] — the immutable build snapshot (ALG-001)
//! - [`normalize`] — the ALG-002 variant text normalization the build
//!   applies to every snapshot text before it reaches a file
//! - [`body`] — the ALG-020…022 assembly of a project's file body
//!   from a snapshot (§9.2)
//! - [`content_build`] — the chunked multi-project content pass over
//!   one snapshot (§9.2 body + ALG-030 managed text + DB-020 hash per
//!   project, DB-021 verdicts; the batching the §12.2 budget needs)
//! - [`color_pick`] — the D-29 rule that assigns a new project the
//!   least-used palette color, hash-tie-broken by project ID
//! - [`heading_range`] — the ALG-060 branch range of a section
//!   heading, as an index range over the ordered items
//! - [`reorder`] — the ALG-061 move: cut a branch (or a one-item
//!   range), insert it at the target position, renumber `position`
//!   from 1
//! - [`levels`] — the ALG-062 level operations: change one
//!   heading's level point-wise, shift every heading of a branch
//!   by ±1
//! - [`gaps`] — the LIB-032 heading-level skips: where the next
//!   heading arrives more than one level deeper, as facts for the
//!   UI warning icon
//! - [`project_validators`] — the §6.9 rules of the editable
//!   project fields: name, label, description, color, file name,
//!   and the DB-042 default label derived from a project's name
//! - [`library_validators`] — the §6.9 rules of the editable library
//!   fields: instruction and heading names, variant name and text

pub mod assembly_b;
pub mod assignment;
pub mod backup_name;
pub mod body;
pub mod color_pick;
pub mod compare;
pub mod content_build;
pub mod dirty;
pub mod extra_files;
pub mod gaps;
pub mod hash;
pub mod heading_range;
pub mod levels;
pub mod library;
pub mod library_validators;
pub mod marker;
pub mod marker_clock;
pub mod markers;
pub mod normalize;
pub mod project;
pub mod project_validators;
pub mod reorder;
pub mod snapshot;
pub mod target_file;
pub mod version_snapshot;
