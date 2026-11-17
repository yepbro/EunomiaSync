//! Repository tree scanning.
//!
//! Planned responsibility (docs/spec.md 5.2): walk the configured roots and
//! discover repositories so that sync and backup-restore operate on the
//! real state on disk. The walk follows spec 9.6 (ALG-050..054):
//! `ignore`'s parallel `WalkBuilder` with `.gitignore` reading disabled —
//! a repository living under an ignored directory must stay visible —
//! symlinks and Windows reparse points not followed, depth from settings,
//! at most `min(4, number of cores)` threads; entry names only, files are
//! never opened. `walkdir` stays available for plain single-threaded
//! traversal outside the repository-scan algorithm (for example, the app's
//! own data directory). Per spec 5.4 (SYS-003), scans take a cancellation
//! token and report progress; the command layer owns the event emission,
//! so the scan API only exposes the hooks.
//!
//! Nothing is implemented yet: this module is part of the crate skeleton
//! and exists to pin the module boundary.
//!
//! # Testing plan
//!
//! Once implemented, tests go through the public API against temporary
//! directory fixtures: exclusion names and system folders, nested
//! repositories found separately (ALG-051), a `.git` directory vs a `.git`
//! file skip (ALG-052), progress and cancellation checked on every entry
//! (ALG-053), and proof that `.gitignore` is not consulted (ALG-050).
