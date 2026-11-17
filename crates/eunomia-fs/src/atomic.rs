//! Atomic file writes with retries.
//!
//! Planned responsibility (docs/spec.md 5.2): replace a target file's
//! content without ever leaving a half-written file behind, following spec
//! 9.5. A temporary file is created in the same folder as the target
//! (`tempfile`; same directory keeps the final move a rename, never a
//! cross-device copy), written and `sync_all`-ed, then swapped in with a
//! single replace (Windows: `MoveFileExW` with replace-existing and
//! write-through); on Unix the parent folder is fsynced, and the written
//! file is re-read and checked against the expected SHA-256. On an error at
//! any step the temporary file is deleted and the original stays unchanged.
//! Sharing violations and access errors are retried up to 3 times with
//! 100/300/900 ms pauses, then `E_FILE_LOCKED`; a hash mismatch is
//! `E_VERIFY_FAILED`.
//!
//! Nothing is implemented yet: this module is part of the crate skeleton
//! and exists to pin the module boundary.
//!
//! # Testing plan
//!
//! Once implemented, tests use temporary directories to cover create-new
//! and overwrite-existing writes, the crash-safety invariant (the target is
//! always the old or the new content, never a partial mix), cleanup of the
//! temporary file on failure, retry behavior ending in `E_FILE_LOCKED`, and
//! the `E_VERIFY_FAILED` hash-mismatch case.
