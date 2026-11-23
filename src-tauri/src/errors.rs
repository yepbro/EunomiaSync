//! The IPC error contract: [`AppError`] and the MVP error-code registry
//! (spec §7.10, §10; UI-011; NFR-006/NFR-018 via UI-020).
//!
//! Every Tauri command returns `Result<T, AppError>` (§7.10), so this
//! type is the error half of the whole IPC surface. It is deliberately
//! **not** a Rust-side error enum: nothing propagates it with `?` and it
//! implements neither `Display` nor `std::error::Error`. It is a wire
//! DTO — a *localization contract*. The Rust side reports what happened
//! (a closed-vocabulary [`ErrorCode`]) plus the values needed to render
//! a message; the UI looks the text up by `message_key` in the locale
//! file and interpolates `params` (UI-020: "backend error texts come as
//! `message_key` and `params`"; NFR-006: the UI never concatenates or
//! displays backend-composed prose). Which code an OS-level failure maps
//! to is context-dependent (a permission error is `E_NO_WRITE_ACCESS`
//! during preflight but `E_BACKUP_FAILED` during the backup phase), so
//! there is deliberately no blanket `From<std::io::Error>` — the
//! command tasks construct the error explicitly and attach the OS facts
//! with [`AppError::with_os_error`].
//!
//! # Fields (§7.10, UI-011)
//!
//! - `code` — the stable `E_*` string from the §10 table. Serialized as
//!   the spec literal (`"E_SYNC_RUNNING"`, …) via per-variant serde
//!   renames; the strings are a wire contract, and the bindings pin in
//!   `errors_tests.rs` keeps `src/bindings.ts` carrying exactly the MVP
//!   set.
//! - `message_key` — the i18n key the UI resolves (NFR-006/NFR-018). A
//!   `String`, not an enum: one code can reach several messages (the
//!   `E_VALIDATION` family names the exact failed field), so the key is
//!   per-error, not per-code. [`ErrorCode::message_key`] provides the
//!   stable default every code maps to (the `errors.<snake_case>`
//!   namespace, matching the UI-020 key style `project.settings.title`);
//!   [`AppError::new`] installs it. The key strings are a contract with
//!   the locale file — renaming one is a coordinated change,
//!   pinned by tests on both sides.
//! - `params` — interpolation values for the localized message: the
//!   "where" of UI-011 (path, project name) and similar user-visible
//!   facts. String-to-string: locale interpolation takes strings, and
//!   keeping the value type uniform keeps the TypeScript side a plain
//!   record. `BTreeMap` so serialization (and with it the §10 log line
//!   and every test) is deterministic.
//! - `details` — the technical half of UI-011 ("why — code and OS
//!   reason"; collapsed into the "Details" section with a copy button).
//!   Structured, not free-form: [`ErrorDetails`] carries the OS error
//!   code, the portable `ErrorKind` name, and the underlying message.
//!   The OS facts live here rather than in `params` because §10 makes
//!   the OS code a first-class fact of every error log line, and
//!   because `params` holds user-visible interpolation values — mixing
//!   the two would force the UI to fish diagnostics out of message
//!   placeholders.
//!
//! # Scope: the 20 MVP codes, exactly
//!
//! The enum carries exactly the twenty MVP codes of the T-013 card /
//! §10 table. The two P1 codes from that table —
//! `E_RESTORE_SOURCE_MISSING` and `E_IMPORT_INVALID` — are **excluded**
//! on purpose: they belong to P1 features (backup restore, library
//! import) and join the registry with their own tasks. The bindings pin
//! enforces the boundary in both directions: all twenty MVP literals
//! present, no twenty-first `E_*` literal.
//!
//! # What this module does *not* do
//!
//! It does not log. §10's "every error is written to the log with its
//! code, project, path, and OS code" is the producing commands' duty
//! (sync, backup, DB — all landed: the `sync_run` pipeline, the
//! backup/recovery commands, and the DB-backed commands' failure edges
//! log through typed helpers), and SEC-008/D-032 pin every tracing
//! event to the `logging.rs` facade, whose `AppError` failure helpers
//! take `&AppError` — this type's fields are the vocabulary (the
//! facade's other helpers log their own types). The module has
//! production callers today: every registered command returns
//! `Result<T, AppError>` (the DB-backed ones through the DB executor,
//! `db::Db::run`), the type is registered with the tauri-specta
//! builder (`Builder::typ`, keeping the type in the export even if a
//! command-set change would otherwise drop it), exported to
//! `src/bindings.ts`, and exercised by the tests below.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use specta::Type;

/// The closed registry of MVP error codes — the `Code` column of the
/// spec §10 table, exactly the twenty codes the T-013 card lists.
///
/// Each variant serializes to its spec §10 literal; those strings are
/// the wire and log contract (§7.10, §10) and are pinned in
/// `src/bindings.ts` by `errors_tests.rs`. The two P1 codes of the §10
/// table are deliberately absent — see the module docs for the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub(crate) enum ErrorCode {
    /// A second sync run, or a project edit during a sync.
    #[serde(rename = "E_SYNC_RUNNING")]
    SyncRunning,
    /// A folder without `.git` is being added as a project.
    #[serde(rename = "E_NOT_A_REPO")]
    NotARepo,
    /// The project folder moved, was deleted, or cannot be read.
    #[serde(rename = "E_PROJECT_UNAVAILABLE")]
    ProjectUnavailable,
    /// No write rights to the folder, or the target file is read-only.
    #[serde(rename = "E_NO_WRITE_ACCESS")]
    NoWriteAccess,
    /// The target path is a directory.
    #[serde(rename = "E_TARGET_IS_DIR")]
    TargetIsDir,
    /// The target path is a dangling symlink.
    #[serde(rename = "E_TARGET_BROKEN_LINK")]
    TargetBrokenLink,
    /// The target path resolves outside the project root.
    #[serde(rename = "E_TARGET_OUTSIDE_ROOT")]
    TargetOutsideRoot,
    /// Scheme B markers are missing, repeated, or out of order.
    #[serde(rename = "E_MARKERS_INVALID")]
    MarkersInvalid,
    /// The scheme B target file is not valid UTF-8.
    #[serde(rename = "E_NOT_UTF8")]
    NotUtf8,
    /// A backup was not created, or failed its verification check.
    #[serde(rename = "E_BACKUP_FAILED")]
    BackupFailed,
    /// No space left on the backups or the project volume.
    #[serde(rename = "E_DISK_FULL")]
    DiskFull,
    /// The target file is locked by another process; retries exhausted.
    #[serde(rename = "E_FILE_LOCKED")]
    FileLocked,
    /// Writing the target file failed.
    #[serde(rename = "E_WRITE_FAILED")]
    WriteFailed,
    /// The post-write hash verification failed.
    #[serde(rename = "E_VERIFY_FAILED")]
    VerifyFailed,
    /// Instruction text contains a scheme B marker line.
    #[serde(rename = "E_CONTENT_RESERVED")]
    ContentReserved,
    /// A field failed validation (spec table 6.9).
    #[serde(rename = "E_VALIDATION")]
    Validation,
    /// The database is busy; `busy_timeout` expired.
    #[serde(rename = "E_DB_LOCKED")]
    DbLocked,
    /// The database failed `quick_check`; recovery screen required.
    #[serde(rename = "E_DB_CORRUPT")]
    DbCorrupt,
    /// The database was created by a newer app version.
    #[serde(rename = "E_DB_TOO_NEW")]
    DbTooNew,
    /// The update check or installation failed.
    #[serde(rename = "E_UPDATE_FAILED")]
    UpdateFailed,
}

impl ErrorCode {
    /// The code's spec §10 literal (`"E_SYNC_RUNNING"`, …) — the wire
    /// form the serde renames produce and the word §10's log line and
    /// UI-011's "why" carry. Single source for logging helpers and
    /// diagnostics: the facade (D-032) reads the literal from
    /// here instead of re-matching twenty strings or going through
    /// serde. The literal test pins `as_str`, the serde output and the
    /// spec table to one another, so the three cannot drift.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            ErrorCode::SyncRunning => "E_SYNC_RUNNING",
            ErrorCode::NotARepo => "E_NOT_A_REPO",
            ErrorCode::ProjectUnavailable => "E_PROJECT_UNAVAILABLE",
            ErrorCode::NoWriteAccess => "E_NO_WRITE_ACCESS",
            ErrorCode::TargetIsDir => "E_TARGET_IS_DIR",
            ErrorCode::TargetBrokenLink => "E_TARGET_BROKEN_LINK",
            ErrorCode::TargetOutsideRoot => "E_TARGET_OUTSIDE_ROOT",
            ErrorCode::MarkersInvalid => "E_MARKERS_INVALID",
            ErrorCode::NotUtf8 => "E_NOT_UTF8",
            ErrorCode::BackupFailed => "E_BACKUP_FAILED",
            ErrorCode::DiskFull => "E_DISK_FULL",
            ErrorCode::FileLocked => "E_FILE_LOCKED",
            ErrorCode::WriteFailed => "E_WRITE_FAILED",
            ErrorCode::VerifyFailed => "E_VERIFY_FAILED",
            ErrorCode::ContentReserved => "E_CONTENT_RESERVED",
            ErrorCode::Validation => "E_VALIDATION",
            ErrorCode::DbLocked => "E_DB_LOCKED",
            ErrorCode::DbCorrupt => "E_DB_CORRUPT",
            ErrorCode::DbTooNew => "E_DB_TOO_NEW",
            ErrorCode::UpdateFailed => "E_UPDATE_FAILED",
        }
    }

    /// The default i18n key for the code — the stable string the UI
    /// resolves in the locale file (`errors.<snake_case>`, following
    /// the UI-020 key style). [`AppError::new`] installs it as the
    /// error's `message_key`; codes with message families
    /// (`E_VALIDATION`) override it per-instance. The returned strings
    /// are part of the frontend contract (NFR-006): renaming one is a
    /// coordinated, test-pinned change, never a drive-by.
    pub(crate) fn message_key(self) -> &'static str {
        match self {
            ErrorCode::SyncRunning => "errors.sync_running",
            ErrorCode::NotARepo => "errors.not_a_repo",
            ErrorCode::ProjectUnavailable => "errors.project_unavailable",
            ErrorCode::NoWriteAccess => "errors.no_write_access",
            ErrorCode::TargetIsDir => "errors.target_is_dir",
            ErrorCode::TargetBrokenLink => "errors.target_broken_link",
            ErrorCode::TargetOutsideRoot => "errors.target_outside_root",
            ErrorCode::MarkersInvalid => "errors.markers_invalid",
            ErrorCode::NotUtf8 => "errors.not_utf8",
            ErrorCode::BackupFailed => "errors.backup_failed",
            ErrorCode::DiskFull => "errors.disk_full",
            ErrorCode::FileLocked => "errors.file_locked",
            ErrorCode::WriteFailed => "errors.write_failed",
            ErrorCode::VerifyFailed => "errors.verify_failed",
            ErrorCode::ContentReserved => "errors.content_reserved",
            ErrorCode::Validation => "errors.validation",
            ErrorCode::DbLocked => "errors.db_locked",
            ErrorCode::DbCorrupt => "errors.db_corrupt",
            ErrorCode::DbTooNew => "errors.db_too_new",
            ErrorCode::UpdateFailed => "errors.update_failed",
        }
    }
}

/// The technical half of an error — what UI-011 collapses into the
/// "Details" section: the OS-level facts behind the failure, attached by
/// [`AppError::with_os_error`]. Every field is optional on the wire:
/// hand-built details (and non-OS failures) may carry any subset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct ErrorDetails {
    /// The OS error code — `std::io::Error::raw_os_error()`: an errno on
    /// Unix, `GetLastError()` on Windows. §10 names it as a first-class
    /// fact of every error log line, and UI-011 shows it as the "why".
    pub(crate) os_code: Option<i32>,
    /// The portable error category — a machine tag, not only
    /// `std::io::ErrorKind`'s Debug name. Three producer families
    /// write it: `with_os_error`/`from_io_error` record
    /// `ErrorKind`'s Debug name (`"PermissionDenied"`,
    /// `"StorageFull"`, …); the shared SQLite base classifier
    /// (`db::sqlite_error_to_app_error`) records rusqlite `ErrorCode`'s
    /// Debug name (`"DatabaseBusy"`, `"SystemIoFailure"`, …) — and
    /// those tags are load-bearing: `Db::run`'s busy-retry cycle
    /// reads exactly `"DatabaseBusy"`/`"DatabaseLocked"` from this
    /// field to decide what to replay (D-050), so a producer must not
    /// "tidy" them into io-kinds; and command code attaches its own
    /// named labels where no OS fact exists (`"OpenerError"`, …).
    /// The fallback that keeps a diagnostic value on platforms and
    /// errors that carry no raw OS code (BKP-005: "OS code **or**
    /// error type").
    pub(crate) os_kind: Option<String>,
    /// The underlying error's `Display` text (e.g.
    /// `"Permission denied (os error 13)"`) — raw diagnostic material
    /// for the copyable Details section, never interpolated into the
    /// localized message (NFR-006).
    pub(crate) os_message: Option<String>,
}

impl ErrorDetails {
    /// Captures the OS-level facts of an I/O failure: the raw OS code
    /// when there is one, plus the portable kind and the message (which
    /// always exist). §10's "OS code" reaches the UI and the log through
    /// this constructor — never through `params`.
    pub(crate) fn from_io_error(error: &std::io::Error) -> Self {
        ErrorDetails {
            os_code: error.raw_os_error(),
            os_kind: Some(format!("{:?}", error.kind())),
            os_message: Some(error.to_string()),
        }
    }
}

/// The error every Tauri command returns in its `Result` (spec §7.10).
/// A localization contract, not a Rust error type — see the module docs
/// for the field-by-field rationale and the no-`From` policy.
///
/// Invariant for direct constructors (the fields are `pub(crate)` for the
/// command tasks that build them): `message_key` must belong to `code` — its
/// [`ErrorCode::message_key`] default, or a deliberate member of the
/// code's message family (`E_VALIDATION`). A key from a different code
/// would render the wrong localized text with the wrong params; the UI
/// trusts the pairing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct AppError {
    /// What happened — one of the twenty MVP codes (spec §10), serialized
    /// as the code's spec literal.
    pub(crate) code: ErrorCode,
    /// The i18n key the UI resolves (NFR-006/NFR-018). Defaults to
    /// [`ErrorCode::message_key`]; overridden per-instance by codes with
    /// message families (e.g. `E_VALIDATION`).
    pub(crate) message_key: String,
    /// Interpolation values for the localized message (UI-011's
    /// "where": path, project, …). Deterministically ordered.
    pub(crate) params: BTreeMap<String, String>,
    /// Technical details for the collapsed "Details" section (UI-011):
    /// the OS code, kind, and message, when the failure came from the
    /// OS. `null` when there is nothing technical to attach.
    pub(crate) details: Option<ErrorDetails>,
}

impl AppError {
    /// Builds the error for `code` with its default `message_key`, no
    /// params, and no details — the base every command extends
    /// (`.params.insert(...)`, [`Self::with_os_error`]).
    pub(crate) fn new(code: ErrorCode) -> Self {
        AppError {
            code,
            message_key: code.message_key().to_owned(),
            params: BTreeMap::new(),
            details: None,
        }
    }

    /// Attaches the OS-level facts of an I/O failure (the raw OS code,
    /// the portable kind, the message) to the error — the §10 requirement
    /// that an OS error reaches the UI and the log with its code. Which
    /// `ErrorCode` the failure maps to stays the caller's decision (see
    /// the module docs for why there is no `From`). Consumed by
    /// `dialog_pick_folder`'s vanish-race mapping since T-073 (D-100).
    pub(crate) fn with_os_error(mut self, error: &std::io::Error) -> Self {
        self.details = Some(ErrorDetails::from_io_error(error));
        self
    }
}

#[cfg(test)]
mod tests {
    //! Behavior tests of the error contract, through the public surface
    //! the commands use: construction, serialization to the §10
    //! wire form, and the OS-code attachment. Expected literals are
    //! pinned as strings derived from the spec table — never computed
    //! from the enum — so a mutated rename or message key fails its own
    //! test (the same trick as the `logging.rs` suite).

    use super::*;
    use std::collections::BTreeSet;

    /// All twenty MVP codes with their spec §10 literals — the card's
    /// list, in table order. This list is the test-side registry: adding
    /// an enum variant without extending it (and the bindings pin) is a
    /// conscious, review-visible act.
    fn all_mvp_codes() -> [(ErrorCode, &'static str); 20] {
        [
            (ErrorCode::SyncRunning, "E_SYNC_RUNNING"),
            (ErrorCode::NotARepo, "E_NOT_A_REPO"),
            (ErrorCode::ProjectUnavailable, "E_PROJECT_UNAVAILABLE"),
            (ErrorCode::NoWriteAccess, "E_NO_WRITE_ACCESS"),
            (ErrorCode::TargetIsDir, "E_TARGET_IS_DIR"),
            (ErrorCode::TargetBrokenLink, "E_TARGET_BROKEN_LINK"),
            (ErrorCode::TargetOutsideRoot, "E_TARGET_OUTSIDE_ROOT"),
            (ErrorCode::MarkersInvalid, "E_MARKERS_INVALID"),
            (ErrorCode::NotUtf8, "E_NOT_UTF8"),
            (ErrorCode::BackupFailed, "E_BACKUP_FAILED"),
            (ErrorCode::DiskFull, "E_DISK_FULL"),
            (ErrorCode::FileLocked, "E_FILE_LOCKED"),
            (ErrorCode::WriteFailed, "E_WRITE_FAILED"),
            (ErrorCode::VerifyFailed, "E_VERIFY_FAILED"),
            (ErrorCode::ContentReserved, "E_CONTENT_RESERVED"),
            (ErrorCode::Validation, "E_VALIDATION"),
            (ErrorCode::DbLocked, "E_DB_LOCKED"),
            (ErrorCode::DbCorrupt, "E_DB_CORRUPT"),
            (ErrorCode::DbTooNew, "E_DB_TOO_NEW"),
            (ErrorCode::UpdateFailed, "E_UPDATE_FAILED"),
        ]
    }

    #[test]
    fn every_mvp_code_serializes_to_its_spec_literal() {
        // Arrange & Act & Assert: each of the twenty codes round-trips
        // through its §10 literal — the wire form the UI and the log
        // see — and `as_str`, the serde output and the spec literal are
        // one and the same string. All twenty literals are distinct (a
        // duplicated rename would silently merge two codes on the wire).
        let mut literals = BTreeSet::new();
        for (code, literal) in all_mvp_codes() {
            assert_eq!(
                code.as_str(),
                literal,
                "`as_str` must return the spec §10 literal"
            );
            assert_eq!(
                serde_json::to_value(code).expect("serializing a code must work"),
                serde_json::json!(literal),
                "`{literal}` must serialize to its spec §10 literal"
            );
            let back: ErrorCode = serde_json::from_value(serde_json::json!(literal))
                .expect("deserializing a spec literal must work");
            assert_eq!(back, code, "`{literal}` must round-trip to its variant");
            literals.insert(literal);
        }
        assert_eq!(
            literals.len(),
            20,
            "the twenty MVP literals must be distinct"
        );
    }

    /// The expected default key per code — an exhaustive match, not a
    /// list: adding an enum variant fails THIS test's compilation until
    /// the key registry is extended, so the Rust-side mapping is
    /// compiler-enforced (the variant↔literal lists above stay the
    /// spec-derived pin; a new variant also trips the bindings count
    /// pin in `errors_tests.rs`).
    fn expected_message_key(code: ErrorCode) -> &'static str {
        match code {
            ErrorCode::SyncRunning => "errors.sync_running",
            ErrorCode::NotARepo => "errors.not_a_repo",
            ErrorCode::ProjectUnavailable => "errors.project_unavailable",
            ErrorCode::NoWriteAccess => "errors.no_write_access",
            ErrorCode::TargetIsDir => "errors.target_is_dir",
            ErrorCode::TargetBrokenLink => "errors.target_broken_link",
            ErrorCode::TargetOutsideRoot => "errors.target_outside_root",
            ErrorCode::MarkersInvalid => "errors.markers_invalid",
            ErrorCode::NotUtf8 => "errors.not_utf8",
            ErrorCode::BackupFailed => "errors.backup_failed",
            ErrorCode::DiskFull => "errors.disk_full",
            ErrorCode::FileLocked => "errors.file_locked",
            ErrorCode::WriteFailed => "errors.write_failed",
            ErrorCode::VerifyFailed => "errors.verify_failed",
            ErrorCode::ContentReserved => "errors.content_reserved",
            ErrorCode::Validation => "errors.validation",
            ErrorCode::DbLocked => "errors.db_locked",
            ErrorCode::DbCorrupt => "errors.db_corrupt",
            ErrorCode::DbTooNew => "errors.db_too_new",
            ErrorCode::UpdateFailed => "errors.update_failed",
        }
    }

    #[test]
    fn message_keys_are_stable_and_follow_the_errors_namespace() {
        // Arrange & Act: the default key of every code, pinned to its
        // literal — the contract with the locale file. The
        // spellings follow UI-020's key style: lowercase, dot-separated
        // namespace, snake_case segment.
        //
        // Assert: the mapping matches, `new` installs it, and no key
        // collides (a collision would make two codes share one locale
        // entry).
        let mut keys = BTreeSet::new();
        for (code, _literal) in all_mvp_codes() {
            let key = expected_message_key(code);
            assert_eq!(code.message_key(), key);
            assert_eq!(AppError::new(code).message_key, key);
            keys.insert(key);
        }
        assert_eq!(keys.len(), 20, "the twenty message keys must be distinct");
    }

    #[test]
    fn serialization_roundtrips_with_params_and_details() {
        // Arrange: a realistic write failure — the code, params carrying
        // UI-011's "where" (path, project), and the OS facts a Windows
        // lock or a permission denial would attach.
        let mut params = BTreeMap::new();
        params.insert("path".to_owned(), "/repo/AGENTS.md".to_owned());
        params.insert("project".to_owned(), "Eunomia".to_owned());
        let error = AppError {
            code: ErrorCode::WriteFailed,
            message_key: "errors.write_failed".to_owned(),
            params,
            details: Some(ErrorDetails {
                os_code: Some(13),
                os_kind: Some("PermissionDenied".to_owned()),
                os_message: Some("Permission denied (os error 13)".to_owned()),
            }),
        };

        // Act: the §7.10 wire form, exactly as the webview receives it.
        let wire = serde_json::to_value(&error).expect("serializing an AppError must work");

        // Assert: the four §7.10 fields under their spec spellings, the
        // code as the §10 literal, and a lossless round-trip.
        assert_eq!(wire["code"], serde_json::json!("E_WRITE_FAILED"));
        assert_eq!(
            wire["message_key"],
            serde_json::json!("errors.write_failed")
        );
        assert_eq!(
            wire["params"],
            serde_json::json!({ "path": "/repo/AGENTS.md", "project": "Eunomia" })
        );
        assert_eq!(wire["details"]["os_code"], serde_json::json!(13));
        let back: AppError =
            serde_json::from_value(wire).expect("deserializing an AppError must work");
        assert_eq!(back, error);
    }

    #[test]
    fn details_serialize_to_null_when_absent() {
        // Arrange & Act: the bare constructor — the shape every
        // non-OS failure takes.
        let wire = serde_json::to_value(AppError::new(ErrorCode::DbTooNew))
            .expect("serializing an AppError must work");

        // Assert: §7.10's "optional details" serializes as `null` (not
        // skipped), keeping the field set uniform for the UI, and the
        // round-trip is lossless.
        assert_eq!(wire["details"], serde_json::json!(null));
        let back: AppError =
            serde_json::from_value(wire).expect("deserializing an AppError must work");
        assert_eq!(back, AppError::new(ErrorCode::DbTooNew));
    }

    #[test]
    fn the_os_error_code_attaches_to_the_serialized_form() {
        // Arrange: a real OS error carrying a code (errno 13 =
        // EACCES), the way `std::fs` surfaces a permission failure.
        let os_error = std::io::Error::from_raw_os_error(13);

        // Act: the constructor the commands run at their boundaries.
        let wire =
            serde_json::to_value(AppError::new(ErrorCode::NoWriteAccess).with_os_error(&os_error))
                .expect("serializing an AppError must work");

        // Assert: the card's criterion — the OS code reaches the
        // serialized representation, under `details` (UI-011's
        // "why"), not among the interpolation params.
        assert_eq!(wire["details"]["os_code"], serde_json::json!(13));
        assert_eq!(wire["params"], serde_json::json!({}));
    }

    #[test]
    fn io_errors_without_an_os_code_fall_back_to_the_kind() {
        // Arrange: an error with no raw OS code — the platform case
        // BKP-005's "OS code or error type" wording covers.
        let errorless = std::io::Error::new(std::io::ErrorKind::InvalidData, "checksum mismatch");

        // Act
        let details = ErrorDetails::from_io_error(&errorless);

        // Assert: no code, but the portable kind and the message keep
        // the Details section diagnostic.
        assert_eq!(details.os_code, None);
        assert_eq!(details.os_kind.as_deref(), Some("InvalidData"));
        assert_eq!(details.os_message.as_deref(), Some("checksum mismatch"));
    }
}
