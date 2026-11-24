//! The orphaned project-backup report (T-332; the owner ruling of
//! 2026-11-24).
//!
//! A `.bak` copy under `backups/` that **no** `backup_records` row
//! mentions has no owner among the app's housekeeping passes: the
//! backup retention (T-114) walks rows, and the phase-2 abort sweep
//! (BKP-005) knows only the copies its own run made. D-091 records
//! how such a file comes to exist — the copy is written straight to
//! its final §5.5 name, so a process death between the copy and its
//! record, or a record transaction that failed after the file was
//! verified, leaves verified bytes nothing points at.
//!
//! The owner's ruling: **never delete them silently.** A copy with
//! no row may be the only surviving evidence of a file (the file the
//! aborted or failed run was trying to protect), so the app makes the debris
//! *visible* and leaves the decision to the user:
//!
//! - **What is shown.** One `warn` line per successful open with two
//!   counts — the orphaned `.bak` files and the project folders
//!   holding at least one — plus the number of folders the pass
//!   could not read. When nothing is orphaned but a folder could not
//!   be listed, the line carries a distinct message: the two counts
//!   are then a lower bound, not a zero verdict. The payload is
//!   numeric only (SEC-008): a
//!   project folder name carries the project-name slug, which is
//!   user text and must not reach diagnostics. The user inspects
//!   `<data dir>/backups/` themselves.
//! - **Who decides.** The user. Nothing here deletes, moves or
//!   rewrites a copy; the pass is read-only by construction (it only
//!   lists directories and compares names against the rows).
//! - **What to do.** Inspect the folder and remove a file by hand
//!   when its content is not needed; the app never reclaims the
//!   space on its own. The manual rule is recorded in DECISIONS.md
//!   (D-190).
//!
//! # Where the report runs
//!
//! At the end of every successful [`crate::db::Db::open`] — the same
//! trigger as the protective-copy retention pass (T-253/D-129): every
//! launch and every recovery reopen goes through that one function,
//! and both facts the report needs (the data root and the open
//! connection) are right there. Like that pass, this one is
//! **best-effort**: a failed query or an unreadable directory logs a
//! `warn` and never fails the open (the D-046/D-054 startup-
//! housekeeping posture); the next open retries the whole report.
//!
//! # What counts as an orphan
//!
//! Exactly the §5.5 layout: a file one level below `backups/`, named
//! `*.bak`, whose relative layout string (`backups/<project
//! folder>/<file>`) no `backup_records.backup_path` cell matches — in
//! **any** status, because a `pruned` or `discarded` row still claims
//! the name it refers to. Everything else in the tree is somebody
//! else's business: user files, a directory squatting on a `.bak`
//! name (the phase-2 suite's own fixture shape), the top-level
//! entries of `backups/`, and non-`.bak` names are all ignored.

use std::collections::HashSet;
use std::io;
use std::path::Path;

use eunomia_store::backup_records;
use rusqlite::Connection;

use crate::data_dirs;
use crate::logging;

/// The §5.5 backup-file suffix (the layout the store's name builders
/// and the retention's deletion guard share).
const BACKUP_SUFFIX: &str = ".bak";

/// The §5.5 relative-path prefix of the project-backup tree
/// (`eunomia_core::backup_name::backup_relative_path` builds the same
/// string, and `backup_records.backup_path` stores it).
const BACKUPS_PATH_PREFIX: &str = "backups/";

/// What one report pass found. The counts are the whole line's
/// payload (SEC-008).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct OrphanReport {
    /// `.bak` files no `backup_records` row claims.
    pub(crate) orphan_files: u64,
    /// Project folders holding at least one of them.
    pub(crate) orphan_folders: u64,
    /// Project folders the pass could not list: the counts above are a
    /// lower bound while this is non-zero, so the line says so.
    pub(crate) unreadable_folders: u64,
}

/// Runs the report once: reads every claimed `backup_path`, scans
/// `backups/`, and logs when there is something to report.
///
/// Called at the end of a successful [`crate::db::Db::open`]; never
/// returns a failure — both halves degrade to a `warn` line (see the
/// module docs).
pub(crate) fn report(data_dir: &Path, connection: &Connection) {
    let known = match backup_records::all_paths(connection) {
        Ok(paths) => paths.into_iter().collect::<HashSet<String>>(),
        Err(error) => {
            logging::backup_orphans_failed(&error);
            return;
        }
    };
    match scan(&data_dirs::backups_dir(data_dir), &known) {
        Ok(found) if found.orphan_files > 0 || found.unreadable_folders > 0 => {
            logging::backup_orphans(&found);
        }
        // The quiet case is the healthy one — no orphans and every
        // folder readable: no line, the same no-op policy the
        // retention passes follow.
        Ok(_) => {}
        Err(error) => logging::backup_orphans_unreadable(&data_dirs::backups_dir(data_dir), &error),
    }
}

/// Counts the orphaned copies under `backups_root`.
///
/// Split from [`report`] so the counting is testable without a
/// database or a subscriber: it is one read-only walk plus set
/// lookups. A missing `backups/` root is the "nothing to report"
/// answer, not an error — a data layout that was never created (the
/// unit tests' temp roots, or a user who deleted the tree per §5.5's
/// tip) has no copies to lose.
///
/// Errors are the root's own listing failure, or a name that is not
/// valid UTF-8 (a layout path is TEXT on the wire, so a non-UTF-8
/// name could never be claimed by a record and the report's counts
/// could not be trusted — the caller logs and retries next open).
/// A project folder that fails to list is *not* an error: it is
/// counted in [`OrphanReport::unreadable_folders`] and the pass keeps
/// judging the folders it could read (the retention pass's
/// partially-unreadable-directory posture).
pub(crate) fn scan(
    backups_root: &Path,
    known: &HashSet<String>,
) -> Result<OrphanReport, io::Error> {
    let entries = match std::fs::read_dir(backups_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(OrphanReport::default());
        }
        Err(error) => return Err(error),
    };
    let mut report = OrphanReport::default();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let folder = text_name(&entry.file_name())?;
        let files = match std::fs::read_dir(entry.path()) {
            Ok(files) => files,
            Err(_) => {
                report.unreadable_folders += 1;
                continue;
            }
        };
        let mut folder_has_orphan = false;
        for file in files {
            let file = file?;
            if !file.file_type()?.is_file() {
                continue;
            }
            let name = text_name(&file.file_name())?;
            if !name.ends_with(BACKUP_SUFFIX) {
                continue;
            }
            let relative = format!("{}{folder}/{name}", BACKUPS_PATH_PREFIX);
            if !known.contains(&relative) {
                report.orphan_files += 1;
                folder_has_orphan = true;
            }
        }
        if folder_has_orphan {
            report.orphan_folders += 1;
        }
    }
    Ok(report)
}

/// A directory entry's name as the TEXT spelling the database uses.
fn text_name(name: &std::ffi::OsStr) -> Result<String, io::Error> {
    name.to_str().map(str::to_owned).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "a backups/ entry name is not valid UTF-8",
        )
    })
}

#[cfg(test)]
mod tests {
    //! The report's counting contract over real directories (the
    //! scan half; the log lines are the facade's own, and the wiring
    //! call has its source pin in `db_tests.rs`).

    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// A throwaway data root carrying a `backups/` folder.
    fn root_with_backups() -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().expect("a temp data root");
        let backups = root.path().join("backups");
        fs::create_dir(&backups).expect("the backups root");
        (root, backups)
    }

    fn write(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("the parent folder");
        }
        fs::write(path, b"copy bytes").expect("the copy");
    }

    fn known(paths: &[&str]) -> HashSet<String> {
        paths.iter().map(|path| (*path).to_owned()).collect()
    }

    const CLAIMED: &str = "backups/0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__my-project/AGENTS.md.20260503T070809042Z.d2a2446b.bak";
    const ORPHAN: &str = "backups/0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__my-project/AGENTS.md.20260504T070809042Z.abcdef12.bak";

    #[test]
    fn a_file_a_row_claims_is_not_reported() {
        let (_root, backups) = root_with_backups();
        write(&backups.join(CLAIMED.trim_start_matches(BACKUPS_PATH_PREFIX)));

        let report = scan(&backups, &known(&[CLAIMED])).expect("the scan answers");

        assert_eq!(report, OrphanReport::default());
    }

    #[test]
    fn a_file_no_row_claims_is_reported() {
        let (_root, backups) = root_with_backups();
        write(&backups.join(ORPHAN.trim_start_matches(BACKUPS_PATH_PREFIX)));

        let report = scan(&backups, &known(&[CLAIMED])).expect("the scan answers");

        // The two fixtures differ only in their name: what makes this
        // one visible is that no row claims it.
        assert_eq!(
            report,
            OrphanReport {
                orphan_files: 1,
                orphan_folders: 1,
                unreadable_folders: 0,
            }
        );
    }

    #[test]
    fn foreign_files_folders_and_names_are_ignored() {
        let (_root, backups) = root_with_backups();
        // A user file at the root, a `.bak` file at the root (the
        // layout puts copies one level deeper — a root-level one is
        // foreign debris), a non-.bak file and a directory squatting
        // on a .bak name inside a project folder.
        write(&backups.join("notes.txt"));
        write(&backups.join("stray.bak"));
        let folder = backups.join("0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__my-project");
        write(&folder.join("AGENTS.md"));
        fs::create_dir(folder.join("AGENTS.md.20260504T070809042Z.abcdef12.bak"))
            .expect("the squatting directory");

        let report = scan(&backups, &known(&[])).expect("the scan answers");

        assert_eq!(report, OrphanReport::default());
    }

    /// A name the database could never carry (its cells are TEXT) is
    /// not guessed around: the pass fails closed instead of trusting
    /// a count it cannot compare.
    #[cfg(unix)]
    #[allow(clippy::print_stderr)] // the filesystem-refusal skip notice, test-only
    #[test]
    fn a_non_utf8_entry_name_fails_the_pass() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let (_root, backups) = root_with_backups();
        let folder = backups.join("0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__my-project");
        fs::create_dir(&folder).expect("the fixture folder");
        let name = OsStr::from_bytes(b"\xFF.bak");
        if fs::write(folder.join(name), b"copy bytes").is_err() {
            // Some filesystems refuse the bytes outright; the branch
            // under test needs the name to exist, so skip with a
            // notice (the mode-bit tests' gate shape).
            eprintln!("skipping the non-UTF-8 arm: the filesystem refuses the name");
            return;
        }

        let error = scan(&backups, &known(&[])).expect_err("the pass must fail closed");

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn folders_are_counted_once_each() {
        let (_root, backups) = root_with_backups();
        let first = "0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__my-project";
        let second = "0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6e__other-project";
        for (folder, name) in [
            (first, "AGENTS.md.20260503T070809042Z.d2a2446b.bak"),
            (first, "AGENTS.md.20260504T070809042Z.abcdef12.bak"),
            (second, "CLAUDE.md.20260503T070809042Z.d2a2446b.bak"),
        ] {
            write(&backups.join(folder).join(name));
        }

        let report = scan(&backups, &known(&[])).expect("the scan answers");

        assert_eq!(
            report,
            OrphanReport {
                orphan_files: 3,
                orphan_folders: 2,
                unreadable_folders: 0,
            }
        );
    }

    #[test]
    fn a_missing_backups_root_is_the_quiet_answer() {
        let root = tempfile::tempdir().expect("a temp data root");

        let report = scan(&root.path().join("backups"), &known(&[])).expect("the scan answers");

        assert_eq!(report, OrphanReport::default());
    }

    // ---- the report half: the facade line over a real database --------

    /// A data root with the layout's `db/`, `db-backups/` and
    /// `backups/` folders, and a migrated database inside where one
    /// `backup_records` row claims [`CLAIMED`] (the store's own
    /// writers, the `backup_records` suite's fixtures).
    fn seeded_data_root(label: &str) -> (tempfile::TempDir, Connection) {
        use eunomia_store::projects::{self, AddedVia, NewProject};
        use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};
        use eunomia_store::storage;
        use eunomia_store::sync_runs::{self, NewSyncRunProject};

        let root = tempfile::tempdir().expect("a temp data root");
        let data = root.path();
        for folder in ["db", "db-backups", "backups"] {
            fs::create_dir(data.join(folder)).expect("a layout folder");
        }
        let mut conn = storage::open_db(&data.join("db").join("eunomiasync.sqlite3"))
            .expect("a fresh database");
        storage::run_migrations(&mut conn, &data.join("db-backups")).expect("a fresh bootstrap");
        let source = format!("/repos/{label}/AGENTS.md");
        let project = projects::create(
            &conn,
            &NewProject {
                path: &format!("/repos/{label}"),
                path_key: &format!("/repos/{label}"),
                name: "Project One",
                label: "P1",
                color: "blue",
                description: "",
                added_via: AddedVia::Manual,
                scheme: Scheme::A,
                base_name: BaseName::AgentsMd,
                extra_local: ExtraFile {
                    enabled: false,
                    position: Position::End,
                },
                extra_project: ExtraFile {
                    enabled: false,
                    position: Position::End,
                },
            },
        )
        .expect("the fixture project")
        .id;
        let run = sync_runs::create_running(
            &conn,
            "2026-11-21T12:00:00.000Z",
            &[NewSyncRunProject {
                project_id: &project,
                target_path: &source,
                build_hash: &"a".repeat(64),
            }],
        )
        .expect("the fixture run")
        .id;
        backup_records::insert(
            &conn,
            &backup_records::NewBackupRecord {
                project_id: &project,
                sync_run_id: Some(&run),
                source_path: &source,
                backup_path: CLAIMED,
                size_bytes: 7,
                sha256_hex: &"b".repeat(64),
                status: backup_records::BackupRecordStatus::Ok,
                created_at: "2026-11-21T12:00:01.123Z",
            },
        )
        .expect("the claimed copy's record");
        (root, conn)
    }

    /// The full pass over a real database: one line, the counts, and
    /// none of the folder names (SEC-008).
    #[test]
    fn the_report_warns_once_with_the_counts_and_no_user_text() {
        let (root, conn) = seeded_data_root("orphans");
        for relative in [CLAIMED, ORPHAN] {
            write(&root.path().join(relative));
        }
        let capture = crate::ipc_tests::WarnCapture::new();
        let subscriber = crate::ipc_tests::warn_capture_subscriber(&capture);

        tracing::subscriber::with_default(subscriber, || report(root.path(), &conn));

        let text = capture.text();
        assert_eq!(
            text.matches("backup copies with no backup_records row")
                .count(),
            1,
            "one line for the pass: {text}"
        );
        assert!(text.contains("orphan_files=1"), "the orphan count: {text}");
        assert!(
            text.contains("orphan_folders=1"),
            "the folder count: {text}"
        );
        // SEC-008: no fragment of the fixture's user data — the
        // project-name slug in the folder, the seeded project's own
        // name and label, its source path, or the layout prefix.
        for forbidden in [
            "my-project",
            "Project One",
            "P1",
            "/repos/",
            "backups/",
            "0193f1a0",
        ] {
            assert!(
                !text.contains(forbidden),
                "SEC-008: {forbidden:?} must not reach the log: {text}"
            );
        }
    }

    /// The healthy tree is silent — the no-op policy the retention
    /// passes follow, and the assertion that the line above is the
    /// report's own, not some other fixture's.
    #[test]
    fn a_clean_tree_logs_nothing() {
        let (root, conn) = seeded_data_root("clean");
        write(&root.path().join(CLAIMED));
        let capture = crate::ipc_tests::WarnCapture::new();
        let subscriber = crate::ipc_tests::warn_capture_subscriber(&capture);

        tracing::subscriber::with_default(subscriber, || report(root.path(), &conn));

        assert_eq!(capture.text(), "", "a claimed-only tree is quiet");
    }

    /// Nothing orphaned, one folder unlistable: the line carries the
    /// lower-bound message, never the "no orphans" reading.
    #[cfg(unix)]
    #[allow(clippy::print_stderr)] // the root-gated skip notice, test-only
    #[test]
    fn an_unreadable_only_pass_warns_the_lower_bound_message() {
        use std::os::unix::fs::PermissionsExt;

        let (root, conn) = seeded_data_root("partial");
        let sealed = root
            .path()
            .join("backups")
            .join("0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__sealed");
        write(&sealed.join("AGENTS.md.20260503T070809042Z.d2a2446b.bak"));
        let untestable = fs::metadata(&sealed).is_ok_and(|metadata| {
            use std::os::unix::fs::MetadataExt;
            metadata.uid() == 0
        });
        if untestable {
            eprintln!("skipping the unreadable-only arm: running as root");
            return;
        }
        fs::set_permissions(&sealed, fs::Permissions::from_mode(0o000))
            .expect("removing all mode bits must work");
        let capture = crate::ipc_tests::WarnCapture::new();
        let subscriber = crate::ipc_tests::warn_capture_subscriber(&capture);
        tracing::subscriber::with_default(subscriber, || report(root.path(), &conn));
        fs::set_permissions(&sealed, fs::Permissions::from_mode(0o755))
            .expect("restoring mode bits must work");

        let text = capture.text();
        assert!(
            text.contains("the orphan backup report could not list every project folder"),
            "the lower-bound message: {text}"
        );
        assert!(text.contains("unreadable_folders=1"), "{text}");
        assert!(
            !text.contains("backup copies with no backup_records row"),
            "the orphan message would misread a zero count: {text}"
        );
    }

    /// A failed records read degrades with its own line and returns —
    /// the open would never see a failure from here.
    #[test]
    fn a_failed_records_read_warns_and_returns() {
        let (root, conn) = seeded_data_root("no-records");
        conn.execute("DROP TABLE backup_records", [])
            .expect("the fixture drops the table");
        let capture = crate::ipc_tests::WarnCapture::new();
        let subscriber = crate::ipc_tests::warn_capture_subscriber(&capture);

        tracing::subscriber::with_default(subscriber, || report(root.path(), &conn));

        let text = capture.text();
        assert_eq!(
            text.matches("orphan backup report could not read the backup records")
                .count(),
            1,
            "one degrade line: {text}"
        );
    }

    /// An unlistable `backups/` root (here: a file squatting on the
    /// directory's name) degrades with the path and the OS cause.
    #[test]
    fn an_unlistable_root_warns_the_path_and_returns() {
        let root = tempfile::tempdir().expect("a temp data root");
        write(&root.path().join("backups"));

        let capture = crate::ipc_tests::WarnCapture::new();
        let subscriber = crate::ipc_tests::warn_capture_subscriber(&capture);
        tracing::subscriber::with_default(subscriber, || {
            let conn = Connection::open_in_memory().expect("an in-memory database");
            // Just enough of the table for `all_paths` to answer
            // (the real schema is the store's; this arm exercises the
            // root, not the read).
            conn.execute_batch(
                "CREATE TABLE backup_records (
                     backup_path TEXT NOT NULL,
                     id TEXT NOT NULL
                 );",
            )
            .expect("the claim table");
            report(root.path(), &conn);
        });

        let text = capture.text();
        assert_eq!(
            text.matches("orphan backup report could not list backups/")
                .count(),
            1,
            "one degrade line: {text}"
        );
        assert!(
            text.contains(&root.path().join("backups").display().to_string()),
            "the line carries the path it could not list (SEC-008 permits paths): {text}"
        );
    }

    /// The partially-unreadable posture: the folder that cannot be
    /// listed is counted, and the folders that could are still judged
    /// (the retention pass's own contract).
    #[cfg(unix)]
    #[allow(clippy::print_stderr)] // the root-gated skip notice, test-only
    #[test]
    fn an_unreadable_folder_is_counted_and_the_readable_ones_still_judged() {
        use std::os::unix::fs::PermissionsExt;

        let (_root, backups) = root_with_backups();
        let sealed = backups.join("0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__sealed");
        write(&sealed.join("AGENTS.md.20260503T070809042Z.d2a2446b.bak"));
        let readable = backups.join("0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6e__readable");
        write(&readable.join("AGENTS.md.20260504T070809042Z.abcdef12.bak"));

        // Mode bits are not enforced for root, so the precondition
        // cannot be set up there (the availability suite's gate).
        let untestable = fs::metadata(&sealed).is_ok_and(|metadata| {
            use std::os::unix::fs::MetadataExt;
            metadata.uid() == 0
        });
        if untestable {
            eprintln!("skipping the unreadable arm: running as root — mode bits are not enforced");
            return;
        }
        fs::set_permissions(&sealed, fs::Permissions::from_mode(0o000))
            .expect("removing all mode bits must work");
        let report = scan(&backups, &known(&[])).expect("the scan answers");
        // Restored even when an assertion below panics, so TempDir's
        // teardown can remove the folder.
        fs::set_permissions(&sealed, fs::Permissions::from_mode(0o755))
            .expect("restoring mode bits must work");

        assert_eq!(
            report,
            OrphanReport {
                orphan_files: 1,
                orphan_folders: 1,
                unreadable_folders: 1,
            }
        );
    }
}
