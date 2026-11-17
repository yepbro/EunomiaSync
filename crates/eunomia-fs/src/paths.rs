//! Path normalization and validation.
//!
//! Planned responsibility (docs/spec.md 5.2): canonicalize and normalize
//! repository paths so that they compare equal across platforms and app
//! restarts — `dunce` keeps canonical Windows paths free of verbatim
//! (`\\?\`) prefixes — and provide the path checks used when validating
//! paths that arrive from the UI (spec 5.6, SEC-003: every UI-supplied
//! path is checked in Rust). The registered-project and session-picked
//! allowlists that feed those checks live in the app and database layers;
//! this module supplies the normalization primitives and checks, not the
//! allowlist policy.
//!
//! Nothing is implemented yet: this module is part of the crate skeleton
//! and exists to pin the module boundary.
//!
//! # Testing plan
//!
//! Once implemented, tests cover canonicalization of relative, absolute,
//! and symlinked paths, the Windows verbatim-prefix case, and rejection of
//! paths outside the registered or session-allowed set.
