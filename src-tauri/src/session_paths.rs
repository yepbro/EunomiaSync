//! The SEC-003 session allowlist and the reusable path check
//! (spec §5.6 SEC-003, §5.3, §7.10 `dialog_pick_folder`; D-009,
//! T-073).
//!
//! SEC-003 (P0): *all paths that come from the UI are checked in
//! Rust — a path must be a registered project or must have been picked
//! in the system dialog in this session.* This module owns both halves
//! of that sentence:
//!
//! - [`SessionPaths`] is the managed state behind "picked in this
//!   session": the canonical forms of the folders the user chose in
//!   the system dialog, remembered by [`crate::commands::
//!   dialog_pick_folder`] the moment the plugin answers. A *session*
//!   is one app process: nothing clears the list — the process exit
//!   is the clear — so every restart starts with an empty allowlist
//!   exactly as the spec word "session" reads.
//! - [`validate_ui_path`] is the reusable check the upcoming
//!   path-consuming commands (`project_add_manual`, `scan_start`, …)
//!   call at their boundary before touching any folder the UI named:
//!   canonicalized match against a registered project (through the
//!   projects repository's own identity, `find_by_path_key`) **or**
//!   presence in the session list. Everything else is a typed
//!   `E_VALIDATION` rejection — nothing is executed for it.
//!
//! # Why canonical forms, and whose identity rule
//!
//! Canonicalization is `eunomia_fs::paths::canonicalize` (DB-030:
//! `dunce::canonicalize`, the real path with symlinks resolved, the
//! verbatim prefix stripped where safe) — a read-only reuse, the crate
//! untouched. A UI path and an allowlisted folder match when they
//! canonicalize to the **same path**: DB-031's aliasing rule (a
//! symlink and its target are one folder) falls out of the
//! canonicalization itself, before any comparison. For the
//! registered-project half the repository's own lookup is the honest
//! oracle: the canonical form is keyed with `eunomia_fs::paths::
//! path_key` (DB-030's per-OS folding) and answered by
//! `eunomia_store::projects::find_by_path_key`, which DB-015 keeps
//! unique across active **and** archived rows — an archived project is
//! still a registered project, and its folder stays allowlisted.
//!
//! # Locking
//!
//! [`SessionPaths`] uses a `std::sync::Mutex`, the inverse of
//! D-037's async-mutex reasoning: the guard is held for a vector
//! lookup/insert and nothing else — no I/O, no `await` — so an async
//! mutex's `Send` guard (the property D-037's use case needed) would
//! buy nothing. A poisoned lock is recovered
//! (`unwrap_or_else(...into_inner())`): the list is plain data that
//! survives a panicking caller, and refusing every later pick over a
//! diagnostic detail would trade a working picker for a clean flag.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager, Runtime};

use crate::commands::managed_db;
use crate::errors::{AppError, ErrorCode, ErrorDetails};

/// The "picked in this session" half of SEC-003: the canonical forms
/// of the folders the user chose in the system dialog during this app
/// process. Managed by the `setup` hook (before anything that could
/// fail — the state is infallible), read by [`validate_ui_path`],
/// written by [`crate::commands::dialog_pick_folder`] only.
///
/// The list grows by user picks alone; duplicates collapse (picking
/// the same folder twice is one entry), so its size is bounded by the
/// folders the user actually picked this session.
#[derive(Default)]
pub(crate) struct SessionPaths(std::sync::Mutex<Vec<PathBuf>>);

impl SessionPaths {
    /// The empty allowlist a fresh app process starts with.
    pub(crate) fn new() -> Self {
        Self(std::sync::Mutex::new(Vec::new()))
    }

    /// Remembers one canonical picked folder (SEC-003's second half).
    /// Duplicate picks collapse: the entry set is what matters, not
    /// the pick count.
    pub(crate) fn remember(&self, canonical: PathBuf) {
        let mut picked = self.locked();
        if !picked.contains(&canonical) {
            picked.push(canonical);
        }
    }

    /// Whether `canonical` (an already-canonicalized path) is one of
    /// this session's picks — the membership check
    /// [`validate_ui_path`] makes before consulting the database.
    /// Comparison goes through `eunomia_fs::paths::path_key`, so ONE
    /// identity rule — DB-030's per-OS folding — governs both halves
    /// of SEC-003 (this one and the repository's `find_by_path_key`);
    /// both sides are canonical forms, so the key is deterministic
    /// and pure to compute. The key builder refuses non-UTF-8 text
    /// (the D-086 addendum): remembered picks are canonical UTF-8 by
    /// the picker's own gate, and a probe that cannot key simply
    /// matches nothing.
    pub(crate) fn contains(&self, canonical: &Path) -> bool {
        let Some(key) = eunomia_fs::paths::path_key(canonical).ok() else {
            return false;
        };
        self.locked()
            .iter()
            .any(|picked| eunomia_fs::paths::path_key(picked).is_ok_and(|keyed| keyed == key))
    }

    /// How many folders this session has picked — the observable size
    /// the tests assert ("a cancelled dialog pick … does not grow the
    /// list"); no production code reads it, hence the allow (the
    /// `file_event` shape: the test suite is the consumer that keeps
    /// this honest until a screen reads the list).
    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        self.locked().len()
    }

    /// The poison-recovery lock of the helpers above (module docs).
    fn locked(&self) -> std::sync::MutexGuard<'_, Vec<PathBuf>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The SEC-003 check every UI path goes through (spec §5.6): `path`
/// is valid if and only if it canonicalizes onto a **registered
/// project** (either state, through the projects repository's
/// `find_by_path_key` identity) **or onto a folder picked in the
/// system dialog this session** ([`SessionPaths`]). On success the
/// canonical form is returned — the caller (`project_add_manual`,
/// `scan_start`, …) stores and probes that form, so canonicalization
/// happens exactly once per path. Every other path is a typed
/// `E_VALIDATION` rejection naming the field, the sent value and the
/// reason (the settings shape), with nothing executed for it.
///
/// A path that cannot be canonicalized is rejected by the same rule,
/// not failed as an OS error: the dialog only returns existing
/// folders and the repository only stores canonical ones, so a
/// non-canonicalizable path cannot be on either allowlist — the
/// honest answer is "not allowlisted", whatever the OS reason. The
/// one typed exception is the D-086 addendum: a path whose canonical
/// form is not valid UTF-8 (or the picker's lossy answer spelling of
/// one) gets the honest ENCODING rejection instead — "unrepresentable"
/// is not "not allowlisted" for a folder the user may genuinely have
/// picked (see [`rejected_unresolvable_ui_path`]); since the addendum
/// the picker itself refuses such picks at the dialog boundary, and
/// this gate is the second line of the same boundary.
///
/// The session list is consulted **before** the database: the pick
/// half of SEC-003 needs no database, so a session-picked folder
/// validates even in recovery mode (no `Db` managed), where the
/// dialog command itself still works. A path on neither half in that
/// mode answers the recorded no-database fallback instead of a
/// validation rejection — the honest state answer (every real
/// consumer of this check needs the database for its own operation
/// and fails before validation would run).
///
/// Blocking work placement (SYS-001): the canonicalization is a
/// metadata walk over live folders and runs on the blocking pool; the
/// repository read runs through the [`Db::run`] executor
/// (`crate::db::Db::run`, D-037) like every database access. The
/// session-list lock is never held across either.
///
/// The first production caller is `project_add_manual` (T-074), which
/// calls this as its input gate before anything else; the upcoming
/// `scan_start`/`project_relink` follow.
pub(crate) async fn validate_ui_path<R: Runtime>(
    app: &AppHandle<R>,
    path: &str,
) -> Result<PathBuf, AppError> {
    // Canonicalize off the async workers (SYS-001): resolving a path
    // is a metadata syscall per component, and a dead mount must not
    // stall an async worker. The refusal is TYPED since the D-086
    // addendum, and the honest answer depends on which half refused:
    // a canonical form that is not valid UTF-8 gets the encoding
    // rejection (the folder may genuinely have been picked — the
    // picker's own boundary already refused it, this is the add-gate
    // twin); a spelling that CONTAINS the replacement character is
    // the picker's lossy ANSWER form, which names no real folder (the
    // D-100 cost, still the honest encoding sentence, never the
    // allowlist one); every other unresolvable path was never picked
    // and is not a registered project's canonical form.
    let sent = PathBuf::from(path);
    let canonical =
        tauri::async_runtime::spawn_blocking(move || eunomia_fs::paths::canonicalize(&sent))
            .await
            .map_err(crate::db::join_failure_to_app_error)?
            .map_err(|source| rejected_unresolvable_ui_path(path, &source))?;
    // The session half first (no database involved — see the module
    // docs for why that order matters in recovery mode).
    if managed_session(app)?.contains(&canonical) {
        return Ok(canonical);
    }
    // The registered-project half: the repository's own identity
    // lookup, keyed the way the add flow will key it (DB-030). The
    // key cannot refuse here — canonicalize's UTF-8 gate produced
    // `canonical` — but the guarantee is enforced, not assumed (the
    // shared encoding envelope answers if it ever does).
    let key = eunomia_fs::paths::path_key(&canonical)
        .map_err(|source| crate::commands::rejected_non_utf8_path(source.path()))?;
    let database = managed_db(app)?;
    let registered = database
        .run(move |transaction| {
            eunomia_store::projects::find_by_path_key(transaction, &key)
                .map_err(crate::commands::projects_store_to_app_error)
        })
        .await?;
    if registered.is_some() {
        Ok(canonical)
    } else {
        Err(rejected_ui_path(path))
    }
}

/// Remembers one canonical picked folder in the managed session
/// allowlist — the write side of SEC-003, called by
/// [`crate::commands::dialog_pick_folder`] only. The missing-state
/// answer is the [`managed_session`] fallback (the `data_root` shape).
pub(crate) fn remember_pick<R: Runtime>(
    app: &AppHandle<R>,
    canonical: PathBuf,
) -> Result<(), AppError> {
    managed_session(app)?.remember(canonical);
    Ok(())
}

/// The session-allowlist state, with the honest missing-state answer
/// (the `data_root` pattern): the state is managed by `setup` before
/// anything that could fail, so its absence is an uninitialized-app
/// bug — the recorded D-037 fallback with a diagnostic naming it, not
/// a panic, and not an empty allowlist silently rejecting (or, worse,
/// accepting) everything. The returned [`tauri::State`] derefs to
/// `&SessionPaths`, so callers use the helpers directly.
fn managed_session<'a, R: Runtime>(
    app: &'a AppHandle<R>,
) -> Result<tauri::State<'a, SessionPaths>, AppError> {
    app.try_state::<SessionPaths>().ok_or_else(|| {
        let mut error = AppError::new(ErrorCode::DbLocked);
        error.details = Some(ErrorDetails {
            os_code: None,
            os_kind: Some("SessionPathsMissing".to_owned()),
            os_message: Some("the session path allowlist was not managed at startup".to_owned()),
        });
        error
    })
}

/// A SEC-003 rejection: `E_VALIDATION` with the path field, the sent
/// value and the reason as params — the settings shape through the
/// shared boundary builder (`rejected_field`, made pub(crate) for
/// exactly this cross-module reuse the way `managed_db` was; `value`
/// is the caller's own input echoed back, never stored data, so
/// SEC-008 is respected). `details` stays empty: the rejection is a
/// policy answer, not an OS failure.
fn rejected_ui_path(path: &str) -> AppError {
    crate::commands::rejected_field(
        "path",
        path,
        "must be a registered project or a folder picked in the dialog this session (SEC-003)",
    )
}

/// The rejection for a path that cannot be resolved at all (the
/// canonicalize-failure half of [`validate_ui_path`]): the recorded
/// D-100 non-UTF-8 seam, carried into the D-086 addendum's typed
/// world. Two shapes get the honest ENCODING rejection (the shared
/// [`crate::commands::rejected_non_utf8_path`] envelope): the typed
/// refusal — the path RESOLVES to a canonical form whose text is not
/// valid UTF-8 — and the picker's lossy ANSWER spelling (a string
/// containing U+FFFD), which names no real folder even for a folder
/// the user did just pick, so the plain SEC-003 sentence would be
/// false and unactionable. Everything else was never picked and is
/// not a registered project's canonical form, and keeps the SEC-003
/// reason. A folder legitimately NAMED with a U+FFFD scalar resolves
/// fine (the bytes match), so the string arm is reached only by the
/// lossy spellings — the one case where the distinction between "not
/// allowlisted" and "unrepresentable" matters.
fn rejected_unresolvable_ui_path(
    path: &str,
    source: &eunomia_fs::paths::CanonicalizeError,
) -> AppError {
    if matches!(source, eunomia_fs::paths::CanonicalizeError::NotUtf8 { .. })
        || path.contains('\u{FFFD}')
    {
        crate::commands::rejected_non_utf8_path(path)
    } else {
        rejected_ui_path(path)
    }
}

#[cfg(test)]
mod tests {
    //! Behavior tests of the session state itself — the allowlist the
    //! IPC suite drives end-to-end through `dialog_pick_folder_with`
    //! and [`validate_ui_path`] (these pins are the state's own:
    //! dedupe and membership, observable through `remember`/`contains`/
    //! `len` — the state's whole public surface).

    use super::*;

    #[test]
    fn remembered_folders_are_members_and_duplicates_collapse() {
        let session = SessionPaths::new();
        let first = PathBuf::from("/home/user/alpha");
        let second = PathBuf::from("/home/user/beta");

        session.remember(first.clone());
        session.remember(second.clone());
        session.remember(first.clone()); // the same folder picked again

        assert!(session.contains(&first), "a remembered folder is a member");
        assert!(
            session.contains(&second),
            "every remembered folder is a member"
        );
        assert!(
            !session.contains(Path::new("/home/user/gamma")),
            "an untouched folder is not a member"
        );
        assert_eq!(
            session.len(),
            2,
            "picking the same folder twice is one entry"
        );
    }

    #[test]
    fn a_fresh_session_is_empty() {
        // The empty allowlist a fresh app process starts with: SEC-003
        // answers "rejected" for everything until the user picks or a
        // project is registered.
        let session = SessionPaths::new();
        assert_eq!(session.len(), 0);
        assert!(!session.contains(Path::new("/home/user/alpha")));
    }

    #[cfg(unix)]
    #[test]
    fn a_probe_that_cannot_key_matches_nothing() {
        // The D-086 addendum's key refusal, one seam out: a probe
        // path whose text is not valid UTF-8 cannot equal any
        // remembered pick (the picker's gate keeps the list UTF-8),
        // so membership honestly answers false instead of failing —
        // the same "cannot key, cannot match" rule the identity seam
        // applies.
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let session = SessionPaths::new();
        session.remember(PathBuf::from("/home/user/alpha"));
        assert!(
            !session.contains(Path::new(OsStr::from_bytes(b"/home/user/repo\xff"))),
            "an unkeyable probe is not a member of anything"
        );
    }
}
