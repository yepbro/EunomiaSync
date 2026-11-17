//! Backup copies before writes.
//!
//! Planned responsibility (docs/spec.md 5.2): create a backup copy of a
//! target file before sync overwrites it, and run the copy checks around
//! backups and restores. Backups follow the data-directory layout (spec
//! 5.5): `backups/<project_id>__<slug>/<file>.<UTC>.<hash8>.bak` under the
//! app data directory, with the project ID and cleaned slug as the name
//! source — never arbitrary user text (SEC-006). A copy counts as created
//! only after a size + SHA-256 check against the source, and no new copy is
//! made when the source hash equals the last backup's (BKP-010); the backup
//! error policy follows BKP-005 (no forced write without a backup).
//!
//! Nothing is implemented yet: this module is part of the crate skeleton
//! and exists to pin the module boundary.
//!
//! # Testing plan
//!
//! Once implemented, tests use temporary directories to cover backup
//! creation and naming, overwrite behavior, copy checks (size and hash,
//! including the damaged-backup case), the same-hash no-new-copy rule, and
//! restore round-trips.
