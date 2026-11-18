//! Behavior tests of the recovery operations (T-024, spec §7.10
//! command notes + §10 `E_DB_CORRUPT` row), driven through the same
//! seams the commands use: [`crate::recovery`] over real files under a
//! `tempfile` data root, with the §5.5 layout created by
//! [`crate::data_dirs::ensure_data_layout`] exactly as the `setup`
//! hook creates it, and every acceptance criterion closed by reopening
//! through [`crate::db::Db::open`] — the operation the recovering app
//! performs next.
//!
//! The card's acceptance criteria map to tests directly:
//!
//! - *from recovery mode one can start with an empty database; the
//!   original file is preserved* —
//!   [`recreate_preserves_the_damaged_file_and_bootstraps_a_working_database`];
//! - *from recovery mode one can start with a chosen copy; the
//!   original file is preserved* —
//!   [`restore_puts_the_chosen_copy_in_place_and_preserves_the_damaged_file`].
//!
//! Everything else pins the mechanics those two depend on: the
//! listing's DTO and order, the name validation (escape attempts), the
//! gate rejections over the chosen copy, sibling preservation, the
//! uniqueness of preserve names, the staging discipline, and the
//! wiring pins that keep the commands and the `setup` hook from
//! silently unwiring (the `config_tests.rs` style).

use std::path::{Path, PathBuf};

use crate::data_dirs;
use crate::db::{Db, DbInitError};
use crate::recovery::{self, RecoveryError};

/// A fixed UTC instant for the preserve-name seam — the tests never
/// depend on the wall clock for a *name*.
fn seam_now() -> jiff::Timestamp {
    "2026-11-18T12:00:00.000Z"
        .parse()
        .expect("the fixed seam timestamp must parse")
}

/// A complete data root: the §5.5 layout, as the `setup` hook leaves
/// it before the database step.
fn data_root() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let root = temp.path().to_path_buf();
    (temp, root)
}

/// The database path of a data root (§5.5).
fn db_path_of(root: &Path) -> PathBuf {
    root.join("db").join("eunomiasync.sqlite3")
}

/// The `db-backups/` directory of a data root (§5.5).
fn backups_dir_of(root: &Path) -> PathBuf {
    root.join("db-backups")
}

/// Writes damaged-database bytes at the database path: a file with a
/// readable SQLite header (so the too-new gate can pass over it) but
/// a smashed b-tree page — the finding-row corruption shape D-045
/// verified empirically. Produced by creating a real database, then
/// destroying its first b-tree page after the header.
fn write_damaged_database(db_path: &Path) -> Vec<u8> {
    let connection =
        rusqlite::Connection::open(db_path).expect("creating the fixture database must work");
    connection
        .execute_batch(
            "CREATE TABLE damaged (id INTEGER PRIMARY KEY); INSERT INTO damaged VALUES (1);",
        )
        .expect("seeding the fixture database must work");
    drop(connection);
    let mut bytes = std::fs::read(db_path).expect("the fixture database must be readable");
    // Page 2 onwards (page size 4096 on the bundled build): smash the
    // first b-tree page's header so `quick_check` reports a finding.
    for byte in bytes.iter_mut().skip(4096).take(32) {
        *byte = 0xFF;
    }
    std::fs::write(db_path, &bytes).expect("writing the damaged fixture must work");
    bytes
}

/// Writes a valid copy under `db-backups/` holding one probe row, the
/// way the store's copy schemes do (a real database file), and returns
/// its path.
fn write_valid_backup(root: &Path, name: &str, marker: &str) -> PathBuf {
    let source = db_path_of(root);
    let connection =
        rusqlite::Connection::open(&source).expect("creating the backup fixture must work");
    connection
        .execute_batch(&format!(
            "CREATE TABLE probe (name TEXT); INSERT INTO probe VALUES ('{marker}');"
        ))
        .expect("seeding the backup fixture must work");
    drop(connection);
    let backup = backups_dir_of(root).join(name);
    std::fs::copy(&source, &backup).expect("copying the backup fixture must work");
    std::fs::remove_file(&source).expect("removing the scratch database must work");
    backup
}

/// The single `pre-recovery-*.sqlite3` file a test expects the
/// operation to have left (the `-wal`/`-shm` siblings a preserve may
/// rename alongside do not count; a second preserved *database* fails:
/// preserve names must not collide silently).
fn preserved_file(root: &Path) -> PathBuf {
    let mut preserved: Vec<PathBuf> = std::fs::read_dir(backups_dir_of(root))
        .expect("db-backups/ must be readable")
        .map(|entry| entry.expect("every entry must be readable").path())
        .filter(|path| {
            path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                name.starts_with("pre-recovery-") && name.ends_with(".sqlite3")
            })
        })
        .collect();
    assert_eq!(
        preserved.len(),
        1,
        "exactly one pre-recovery file must exist, found: {preserved:?}"
    );
    preserved.remove(0)
}

/// Asserts the §7.10 wire form of a validation rejection.
fn assert_validation(error: &RecoveryError, expected_name: &str, expected_reason: &str) {
    let RecoveryError::InvalidBackupName { name, reason } = error else {
        panic!("expected a name-validation rejection, got: {error:?}");
    };
    assert_eq!(name, expected_name);
    assert_eq!(reason.as_str(), expected_reason);
    let wire = serde_json::to_value(error.app_error()).expect("the error must serialize");
    assert_eq!(wire["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(wire["message_key"], serde_json::json!("errors.validation"));
    assert_eq!(wire["params"]["name"], serde_json::json!(expected_name));
    assert_eq!(wire["params"]["reason"], serde_json::json!(expected_reason));
}

// ---- The card's acceptance criteria ---------------------------------------

#[test]
fn recreate_preserves_the_damaged_file_and_bootstraps_a_working_database() {
    // Arrange: a data root whose database is damaged — the recovery
    // mode state (the startup `Db::open` rejected it as corrupt).
    let (temp, root) = data_root();
    let db_path = db_path_of(&root);
    let damaged_bytes = write_damaged_database(&db_path);
    assert!(
        matches!(Db::open(&root), Err(DbInitError::Corrupt { .. })),
        "the fixture must be the recovery-mode state"
    );

    // Act: the file mechanics of `db_recreate`.
    let preserved = recovery::recreate_files(&root, seam_now())
        .expect("recreate must preserve the damaged file")
        .expect("a damaged file existed, so one must have been preserved");

    // Assert — the card's criterion: the original file is preserved
    // (renamed, byte-identical, under the §7.10 name shape, mode 0600
    // per D-048) …
    assert_eq!(preserved.from, db_path);
    let preserved_path = preserved_file(&root);
    assert_eq!(preserved_path, preserved.to);
    assert_eq!(
        preserved_path
            .file_name()
            .expect("a file name exists")
            .to_string_lossy(),
        "pre-recovery-20261118T120000000Z.sqlite3",
        "the preserve name is the §7.10 shape with the store's stamp"
    );
    assert_eq!(
        std::fs::read(&preserved_path).expect("the preserved file must be readable"),
        damaged_bytes,
        "the damaged database must be preserved byte-identically — never deleted"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&preserved_path)
            .expect("the preserved file must be statable")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o600,
            "the preserved copy is DB data at rest (SEC-007)"
        );
    }
    // … no sibling debris was left in db/ …
    assert!(
        !db_path.with_file_name("eunomiasync.sqlite3-wal").exists()
            && !db_path.with_file_name("eunomiasync.sqlite3-shm").exists(),
        "no sidecar siblings may linger in db/"
    );

    // … and the app can start with the empty database: the normal
    // initialization over the same root succeeds (the reopened
    // database is empty).
    let database = Db::open(&root).expect("the fresh bootstrap must open");
    let marker_count = tauri::async_runtime::block_on(database.run(|tx| {
        tx.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'probe'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("the reopened database must answer queries");
    assert_eq!(
        marker_count, 0,
        "the recreated database is empty — the damaged file's table did not survive into it"
    );
    drop(temp);
}

#[test]
fn restore_puts_the_chosen_copy_in_place_and_preserves_the_damaged_file() {
    // Arrange: a valid daily copy holding a marker row, then a damaged
    // database over the live path — the state after ERR-001 has run
    // for days and the database then got damaged.
    let (temp, root) = data_root();
    let db_path = db_path_of(&root);
    let backup = write_valid_backup(&root, "daily-20261117.sqlite3", "restore-marker");
    let damaged_bytes = write_damaged_database(&db_path);

    // Act: the file mechanics of `db_restore_backup`.
    let preserved = recovery::restore_files(&root, "daily-20261117.sqlite3", seam_now(), |_| {})
        .expect("restore must apply the chosen copy")
        .expect("a damaged file existed, so one must have been preserved");

    // Assert — the card's criterion: the copy's data is the database
    // now …
    let database = Db::open(&root).expect("the restored database must open");
    let marker: String = tauri::async_runtime::block_on(database.run(|tx| {
        tx.query_row("SELECT name FROM probe", [], |row| row.get(0))
            .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("the restored database must answer queries");
    assert_eq!(
        marker, "restore-marker",
        "the chosen copy's data is in place"
    );
    // … the damaged file is preserved byte-identically …
    assert_eq!(
        std::fs::read(&preserved.to).expect("the preserved file must be readable"),
        damaged_bytes,
        "the damaged database must be preserved byte-identically — never deleted"
    );
    // … and the chosen copy itself survives untouched (a copy is
    // never consumed by a restore).
    assert!(
        backup.exists(),
        "the chosen copy must stay in db-backups/ after the restore"
    );
    let connection =
        rusqlite::Connection::open(&backup).expect("the chosen copy must still be a database");
    let marker: String = connection
        .query_row("SELECT name FROM probe", [], |row| row.get(0))
        .expect("the chosen copy must still hold its data");
    assert_eq!(marker, "restore-marker");
    drop(temp);
}

// ---- The listing ----------------------------------------------------------

#[test]
fn the_listing_carries_name_size_and_utc_mtime_newest_first() {
    // Arrange: three copies with known sizes and distinct mtimes, plus
    // noise the listing must skip — a `.tmp` staging leftover, a
    // directory, and a pre-recovery file (a preserved damaged database
    // is listed too: it is a file in the directory, and picking it
    // answers with the honest gate rejection, not a hidden row).
    let (temp, root) = data_root();
    let backups = backups_dir_of(&root);
    let older = write_valid_backup(&root, "daily-20261116.sqlite3", "older");
    let newer = write_valid_backup(&root, "daily-20261117.sqlite3", "newer");
    let prefix = write_valid_backup(
        &root,
        "pre-migration-v0-20261115T000000000Z.sqlite3",
        "prefix",
    );
    std::fs::write(
        backups.join("daily-20261118.sqlite3.tmp"),
        b"staging debris",
    )
    .expect("writing staging debris must work");
    std::fs::write(
        backups.join("pre-recovery-20261117T000000000Z.sqlite3-wal"),
        b"a preserve sibling, not a copy",
    )
    .expect("writing a preserve sibling must work");
    std::fs::write(backups.join("notes.txt"), b"a foreign file")
        .expect("writing a foreign file must work");
    std::fs::create_dir(backups.join("not-a-copy-dir")).expect("creating a directory must work");
    // Deterministic, distinct mtimes: the copy written first is the
    // oldest (the file system's own granularity must not decide the
    // order the screen shows).
    set_mtime(&older, "2026-11-16T08:00:00Z");
    set_mtime(&newer, "2026-11-17T09:30:00Z");
    set_mtime(&prefix, "2026-11-15T23:00:00Z");

    // Act
    let listed = recovery::list_backups(&backups).expect("the listing must succeed");

    // Assert: exactly the three real copies, newest first, with the
    // DTO fields the screen renders.
    let names: Vec<&str> = listed.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "daily-20261117.sqlite3",
            "daily-20261116.sqlite3",
            "pre-migration-v0-20261115T000000000Z.sqlite3"
        ],
        "newest first; staging debris, preserve siblings, foreign files \
         and directories are not copies"
    );
    let newest = &listed[0];
    assert_eq!(
        newest.size_bytes as u64,
        std::fs::metadata(&newer).unwrap().len()
    );
    assert_eq!(
        newest.modified_utc, "2026-11-17T09:30:00Z",
        "the mtime is UTC ISO-8601"
    );
    drop(temp);
}

#[test]
fn an_empty_backups_directory_lists_nothing() {
    // The honest empty state: no copies, no error — the screen shows
    // "start with an empty database" over an empty list.
    let (temp, root) = data_root();
    let listed =
        recovery::list_backups(&backups_dir_of(&root)).expect("an empty dir lists nothing");
    assert_eq!(listed, Vec::new());
    drop(temp);
}

#[test]
fn a_tie_in_mtime_breaks_by_name() {
    // Determinism: equal mtimes must not make the order depend on the
    // directory iterator's whims.
    let (temp, root) = data_root();
    let a = write_valid_backup(&root, "daily-b.sqlite3", "b");
    let b = write_valid_backup(&root, "daily-a.sqlite3", "a");
    set_mtime(&a, "2026-11-17T09:30:00Z");
    set_mtime(&b, "2026-11-17T09:30:00Z");
    let listed = recovery::list_backups(&backups_dir_of(&root)).expect("the listing must succeed");
    let names: Vec<&str> = listed.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, ["daily-a.sqlite3", "daily-b.sqlite3"]);
    drop(temp);
}

// ---- Selection validation (SEC-009's spirit) -------------------------------

#[test]
fn escape_attempts_are_rejected_as_validation_before_any_file_is_touched() {
    // Arrange: a healthy copy that a successful escape would reach,
    // and a damaged database that must remain exactly where it was.
    let (temp, root) = data_root();
    write_valid_backup(&root, "daily-20261117.sqlite3", "escape");
    let db_path = db_path_of(&root);
    let damaged_bytes = write_damaged_database(&db_path);

    // Act & Assert: every path-shaped spelling the UI could send is
    // rejected as E_VALIDATION — a name is a base name, never a path —
    // and a preserve sibling or a foreign file is not a copy, even
    // though it is a plain name that exists.
    std::fs::write(
        backups_dir_of(&root).join("pre-recovery-20261118T120000000Z.sqlite3-wal"),
        b"stale sibling frames",
    )
    .expect("writing a preserve sibling must work");
    std::fs::write(backups_dir_of(&root).join("notes.txt"), b"a foreign file")
        .expect("writing a foreign file must work");
    for attempt in [
        "../daily-20261117.sqlite3",
        "..\\daily-20261117.sqlite3",
        "/etc/passwd",
        "..",
        ".",
        "",
        "db-backups/daily-20261117.sqlite3",
        "daily-20261117.sqlite3.tmp",
        "pre-recovery-20261118T120000000Z.sqlite3-wal",
        "notes.txt",
        "no-such-copy.sqlite3",
    ] {
        let error = recovery::restore_files(&root, attempt, seam_now(), |_| {})
            .expect_err("a path-shaped or non-copy selection must be rejected");
        assert!(
            matches!(error, RecoveryError::InvalidBackupName { .. }),
            "attempt {attempt:?} must be a validation rejection, got: {error:?}"
        );
    }
    // The two reason vocabularies the wire params carry.
    let error = recovery::restore_files(&root, "../escape", seam_now(), |_| {}).unwrap_err();
    assert_validation(&error, "../escape", "not a plain file name");
    let error =
        recovery::restore_files(&root, "no-such-copy.sqlite3", seam_now(), |_| {}).unwrap_err();
    assert_validation(&error, "no-such-copy.sqlite3", "no such copy");
    let error = recovery::restore_files(&root, "daily-20261117.sqlite3.tmp", seam_now(), |_| {})
        .unwrap_err();
    assert_validation(
        &error,
        "daily-20261117.sqlite3.tmp",
        "a staging file, not a copy",
    );
    let error = recovery::restore_files(
        &root,
        "pre-recovery-20261118T120000000Z.sqlite3-wal",
        seam_now(),
        |_| {},
    )
    .unwrap_err();
    assert_validation(
        &error,
        "pre-recovery-20261118T120000000Z.sqlite3-wal",
        "not a database copy",
    );

    // Nothing was touched: the damaged database is byte-identical and
    // no preserve file exists (the `-wal` fixture written above is a
    // test input, not a preserve: it survives unreplaced).
    assert_eq!(
        std::fs::read(&db_path).expect("the damaged file must still exist"),
        damaged_bytes,
        "a rejected selection must not touch the database"
    );
    assert!(
        std::fs::read_dir(backups_dir_of(&root))
            .expect("db-backups/ must be readable")
            .filter_map(|entry| entry.ok())
            .all(|entry| {
                !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("pre-recovery-")
                    || entry.file_name().to_string_lossy().ends_with("-wal")
            }),
        "a rejected selection must not preserve anything"
    );
    drop(temp);
}

#[test]
fn a_damaged_copy_is_rejected_as_corrupt_before_the_live_file_is_touched() {
    // Arrange: a damaged copy in db-backups/ and a damaged live
    // database whose bytes are the assertion's reference.
    let (temp, root) = data_root();
    let db_path = db_path_of(&root);
    let damaged_bytes = write_damaged_database(&db_path);
    write_damaged_database(&backups_dir_of(&root).join("daily-20261117.sqlite3"));

    // Act
    let error = recovery::restore_files(&root, "daily-20261117.sqlite3", seam_now(), |_| {})
        .expect_err("a damaged copy must be rejected");

    // Assert: the §10 wire form of the gate's verdict, and nothing
    // touched — the live file keeps its bytes, nothing was preserved.
    let RecoveryError::DbInit(init) = &error else {
        panic!("expected a gate rejection, got: {error:?}");
    };
    assert!(matches!(init.as_ref(), DbInitError::Corrupt { .. }));
    let wire = serde_json::to_value(error.app_error()).expect("the error must serialize");
    assert_eq!(wire["code"], serde_json::json!("E_DB_CORRUPT"));
    assert_eq!(wire["message_key"], serde_json::json!("errors.db_corrupt"));
    assert_eq!(
        wire["params"]["path"],
        serde_json::json!(
            backups_dir_of(&root)
                .join("daily-20261117.sqlite3")
                .display()
                .to_string()
        )
    );
    assert!(
        wire["params"]["detail"]
            .as_str()
            .is_some_and(|detail| !detail.is_empty()),
        "SQLite's own evidence travels as a param"
    );
    assert_eq!(
        std::fs::read(&db_path).expect("the damaged file must still exist"),
        damaged_bytes
    );
    drop(temp);
}

#[test]
fn a_copy_from_a_newer_app_is_rejected_as_too_new() {
    // Arrange: a copy with `user_version` above this app's set (the
    // T-F11 "database by hand" fixture) — restoring it would trade a
    // damaged database for an unopenable one.
    let (temp, root) = data_root();
    let newer_version = eunomia_store::storage::latest_schema_version() as i64 + 3;
    let copy = backups_dir_of(&root).join("daily-20261117.sqlite3");
    let connection =
        rusqlite::Connection::open(&copy).expect("creating the fixture copy must work");
    connection
        .pragma_update(None, "user_version", newer_version)
        .expect("setting the fixture version must work");
    drop(connection);

    // Act
    let error = recovery::restore_files(&root, "daily-20261117.sqlite3", seam_now(), |_| {})
        .expect_err("a too-new copy must be rejected");

    // Assert: the same typed verdict the startup gate produces, before
    // anything is touched.
    let wire = serde_json::to_value(error.app_error()).expect("the error must serialize");
    assert_eq!(wire["code"], serde_json::json!("E_DB_TOO_NEW"));
    assert_eq!(wire["message_key"], serde_json::json!("errors.db_too_new"));
    assert!(
        !db_path_of(&root).exists(),
        "the (here absent) live file was not created by a rejected restore"
    );
    drop(temp);
}

// ---- The preserve mechanics ------------------------------------------------

#[test]
fn sidecar_siblings_are_preserved_next_to_the_damaged_file() {
    // Arrange: a damaged database with stale `-journal`/`-wal`/`-shm`
    // siblings — the crash-leftover state. A sibling left in `db/`
    // would be presented to the *next* database file that opens there
    // (a hot journal rolled back into a fresh bootstrap, a stale WAL
    // applied to it), so the preserve step must move them out with the
    // damaged file.
    //
    // Driven through the `preserve_damaged` seam directly: the
    // composed `recreate_files` runs its guard gates (bare SQLite
    // reads over the live file) first, and SQLite's own crash
    // recovery during those reads already removes siblings it
    // considers bogus — the D-040/D-045 accepted boundary (verified
    // empirically: a fake `-journal`/`-wal`/`-shm` set does not
    // survive `reject_too_new`). The preserve's sibling handling is
    // the belt-and-suspenders for what the reads leave behind (a
    // `-wal` SQLite could not checkpoint, a `-shm` it could not
    // map); its contract is pinned here, at the seam that owns it.
    let (temp, root) = data_root();
    let db_path = db_path_of(&root);
    let damaged_bytes = write_damaged_database(&db_path);
    for (sibling, bytes) in [
        (
            "eunomiasync.sqlite3-journal",
            b"stale journal pages".as_slice(),
        ),
        ("eunomiasync.sqlite3-wal", b"stale wal frames"),
        ("eunomiasync.sqlite3-shm", b"stale shm index"),
    ] {
        std::fs::write(db_path.with_file_name(sibling), bytes)
            .expect("writing a stale sibling must work");
    }
    // A loosened sibling: the best-effort re-tighten must reach it
    // (they hold database frames; D-048's at-rest guarantee follows
    // the files into `db-backups/`).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            db_path.with_file_name("eunomiasync.sqlite3-wal"),
            std::fs::Permissions::from_mode(0o644),
        )
        .expect("loosening the sibling must work");
    }

    // Act
    recovery::preserve_damaged(&db_path, &backups_dir_of(&root), seam_now())
        .expect("the preserve step must succeed")
        .expect("a damaged file existed");

    // Assert: `db/` holds no database artifacts at all, and the
    // siblings were renamed *alongside* the preserved file (their
    // forensic content survives, keyed to the preserved name).
    assert!(!db_path.exists(), "the damaged file is gone from db/");
    for sibling in [
        "eunomiasync.sqlite3-journal",
        "eunomiasync.sqlite3-wal",
        "eunomiasync.sqlite3-shm",
    ] {
        assert!(
            !db_path.with_file_name(sibling).exists(),
            "db/ must not keep `{sibling}`"
        );
    }
    let preserved = preserved_file(&root);
    for (suffix, bytes) in [
        ("-journal", b"stale journal pages".as_slice()),
        ("-wal", b"stale wal frames"),
        ("-shm", b"stale shm index"),
    ] {
        let sibling = preserved.with_file_name(format!(
            "{}{suffix}",
            preserved.file_name().unwrap().to_string_lossy()
        ));
        assert_eq!(
            std::fs::read(&sibling)
                .unwrap_or_else(|_| panic!("the {suffix} sibling must be preserved")),
            bytes
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&sibling)
                .expect("the sibling must be statable")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(
                mode, 0o600,
                "the loosened {suffix} sibling must be re-tightened (SEC-007)"
            );
        }
    }
    assert_eq!(
        std::fs::read(&preserved).expect("the preserved file must be readable"),
        damaged_bytes
    );
    drop(temp);
}

#[test]
fn siblings_are_cleared_even_when_the_main_file_is_missing() {
    // The hand-deleted `db/` variant that still carries crash-leftover
    // siblings: nothing to preserve, but the siblings must not stay
    // where the next bootstrap would find them.
    let (temp, root) = data_root();
    let db_path = db_path_of(&root);
    std::fs::write(
        db_path.with_file_name("eunomiasync.sqlite3-wal"),
        b"stale wal frames",
    )
    .expect("writing a stale -wal must work");

    let preserved = recovery::recreate_files(&root, seam_now())
        .expect("recreate over a missing main file must succeed");

    assert_eq!(preserved, None, "there is no main file to preserve");
    assert!(
        !db_path.with_file_name("eunomiasync.sqlite3-wal").exists(),
        "the stale sibling must not survive in db/"
    );
    let backups = backups_dir_of(&root);
    let moved: Vec<_> = std::fs::read_dir(&backups)
        .expect("db-backups/ must be readable")
        .map(|entry| {
            entry
                .expect("every entry must be readable")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with("-wal"))
        .collect();
    assert_eq!(
        moved,
        ["pre-recovery-20261118T120000000Z.sqlite3-wal"],
        "the sibling was moved out under the preserve name"
    );
    drop(temp);
}

#[test]
fn a_same_millisecond_collision_takes_a_suffixed_name() {
    // Windows refuses to rename onto an existing target, and two
    // recoveries inside one stamp must both keep their file — the
    // uniqueness that makes the preserve correct cross-platform.
    let (temp, root) = data_root();
    let first = write_valid_backup(&root, "daily-20261117.sqlite3", "first");
    write_damaged_database(&db_path_of(&root));

    let preserved_one = recovery::recreate_files(&root, seam_now())
        .expect("the first preserve must succeed")
        .expect("a damaged file existed");
    // Rebuild a damaged file, then preserve again at the same instant.
    write_damaged_database(&db_path_of(&root));
    let preserved_two = recovery::recreate_files(&root, seam_now())
        .expect("the second preserve must succeed")
        .expect("a second damaged file existed");

    assert_ne!(preserved_one.to, preserved_two.to, "the names differ");
    assert!(
        preserved_two
            .to
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("pre-recovery-20261118T120000000Z-")),
        "the colliding name takes a -N suffix, got: {:?}",
        preserved_two.to
    );
    assert!(preserved_one.to.exists() && preserved_two.to.exists() && first.exists());
    drop(temp);
}

#[test]
fn a_missing_database_file_is_preserved_as_nothing() {
    // The hand-deleted `db/` state: nothing to preserve, the fresh
    // bootstrap is still the right outcome (and the preserve step must
    // not invent a file).
    let (temp, root) = data_root();
    let preserved = recovery::recreate_files(&root, seam_now())
        .expect("recreate over a missing database must succeed");
    assert_eq!(preserved, None);
    assert!(
        Db::open(&root).is_ok(),
        "the bootstrap over the missing file works"
    );
    drop(temp);
}

// ---- The replace mechanics -------------------------------------------------

#[test]
fn the_replacement_leaves_no_staging_file_and_survives_a_stale_one() {
    // Arrange: stale staging debris of an earlier crashed run, plus a
    // valid copy to restore. The live file can be absent here: the
    // stage+publish pair's contract is about the copy → path step,
    // and the restore runs it right after a preserve that found
    // nothing.
    let (temp, root) = data_root();
    let db_path = db_path_of(&root);
    let staging = db_path.with_file_name("eunomiasync.sqlite3.tmp");
    std::fs::write(&staging, b"stale staging debris").expect("writing stale staging must work");
    write_valid_backup(&root, "daily-20261117.sqlite3", "stale-marker");
    let backup = backups_dir_of(&root).join("daily-20261117.sqlite3");
    recovery::stage_copy(&backup, &db_path).expect("the copy must be staged");
    recovery::publish_copy(&db_path).expect("the staged copy must be published");

    // Assert: the debris was swept, the database is the copy's bytes,
    // and no `.tmp` remains.
    assert!(!staging.exists(), "staging debris must be swept");
    assert_eq!(
        std::fs::read(&db_path).expect("the database must be readable"),
        std::fs::read(&backup).expect("the copy must be readable"),
        "the database is a byte copy of the chosen backup"
    );
    drop(temp);
}

#[test]
#[cfg(unix)]
fn a_failed_stage_leaves_the_live_file_in_place() {
    // The round-1 review's failure-window criterion: the copy step is
    // the most failure-prone one (full disk, permissions), so it runs
    // while the live file is still in place — its failure must leave
    // `db/` exactly as it was, with nothing preserved and nothing
    // staged. The writable-rights fixture is the portable stand-in
    // for every staging failure; root bypasses permission bits, so the
    // test skips there (the same gate as the store's suite).
    use std::os::unix::fs::PermissionsExt;
    let (temp, root) = data_root();
    if running_as_root_is_untestable(temp.path()) {
        return;
    }
    let db_path = db_path_of(&root);
    // The copy first, the damaged file after — the helper's scratch
    // database lives at the same path.
    write_valid_backup(&root, "daily-20261117.sqlite3", "window");
    let damaged_bytes = write_damaged_database(&db_path);

    // No write rights in `db/`: the staged copy cannot be created.
    std::fs::set_permissions(root.join("db"), std::fs::Permissions::from_mode(0o555))
        .expect("removing the db directory's write rights must work");
    let error = recovery::restore_files(&root, "daily-20261117.sqlite3", seam_now(), |_| {})
        .expect_err("the unwritable db directory must fail the stage");

    // Assert: the failure is the honest permission verdict, and the
    // live file is untouched — not preserved away, not replaced; the
    // next startup would still see the damaged database and the
    // recovery screen (not a silent empty bootstrap).
    let wire = serde_json::to_value(error.app_error()).expect("the error must serialize");
    assert_eq!(wire["code"], serde_json::json!("E_NO_WRITE_ACCESS"));
    assert_eq!(
        std::fs::read(&db_path).expect("the damaged file must still exist"),
        damaged_bytes,
        "a failed stage must leave the live file byte-identical"
    );
    assert!(
        !backups_dir_of(&root)
            .join("pre-recovery-20261118T120000000Z.sqlite3")
            .exists(),
        "a failed stage must not preserve anything"
    );
    drop(temp);
}

/// Root (uid 0) bypasses permission-bit fixtures, so a chmod-based
/// unwritable-directory test proves nothing there — the same gate as
/// the store's suite (the temp directory was created by this process,
/// so its owner uid is the effective uid).
#[cfg(unix)]
fn running_as_root_is_untestable(temp: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(temp)
        .expect("the temp directory must be statable")
        .uid()
        == 0
}

#[test]
fn recreate_refuses_over_a_healthy_unmanaged_database() {
    // The `db_recreate` guard: "start with an empty database" is the
    // screen's last resort. A healthy file at the database path means
    // a restore placed a copy and the reopen failed — the honest
    // answers are retrying the restore or restarting the app, never
    // silently discarding a working database (the round-1 review's
    // double-failure window).
    let (temp, root) = data_root();
    let db_path = db_path_of(&root);
    write_valid_backup(&root, "daily-20261117.sqlite3", "healthy");
    let healthy_bytes = std::fs::read(backups_dir_of(&root).join("daily-20261117.sqlite3"))
        .expect("the fixture copy must be readable");
    std::fs::copy(
        backups_dir_of(&root).join("daily-20261117.sqlite3"),
        &db_path,
    )
    .expect("placing the healthy fixture must work");

    let error = recovery::recreate_files(&root, seam_now())
        .expect_err("a healthy unmanaged database must be refused");

    // Assert: the E_VALIDATION wire form with the reason and path,
    // and nothing touched.
    let RecoveryError::RefusedHealthy { .. } = &error else {
        panic!("expected the healthy-refusal, got: {error:?}");
    };
    let wire = serde_json::to_value(error.app_error()).expect("the error must serialize");
    assert_eq!(wire["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(
        wire["params"]["reason"],
        serde_json::json!("a working database is already in place — restart the app")
    );
    assert_eq!(
        wire["params"]["path"],
        serde_json::json!(db_path.display().to_string())
    );
    assert_eq!(
        std::fs::read(&db_path).expect("the healthy file must still exist"),
        healthy_bytes,
        "the refusal must not touch the file"
    );
    drop(temp);
}

#[test]
#[cfg(unix)]
fn the_restored_database_file_is_owner_only() {
    // D-048 converge: the artifact this task creates (the replaced
    // database file) enforces its mode at creation — the staged copy
    // is secured *before* publication. Windows inherits the profile's
    // rights (SEC-007).
    let (temp, root) = data_root();
    let db_path = db_path_of(&root);
    write_valid_backup(&root, "daily-20261117.sqlite3", "mode");
    recovery::restore_files(&root, "daily-20261117.sqlite3", seam_now(), |_| {})
        .expect("restore must succeed");
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&db_path)
        .expect("the replaced database must be statable")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "the replaced database file is 0600 (SEC-007)");
    drop(temp);
}

// ---- Wiring pins (the config_tests.rs style) --------------------------------

#[test]
fn setup_manages_the_data_root_before_the_database_step() {
    // The recovery commands depend on the managed `DataRoot`; the
    // database open is the step that can fail into recovery mode, so
    // the root must be managed before it.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    let markers = [
        "data_dirs::ensure_data_layout(&data_dir)?;",
        "app.manage(data_dirs::DataRoot(data_dir.clone()));",
        "db::Db::open(&data_dir)",
    ];
    let positions: Vec<usize> = markers
        .iter()
        .map(|marker| {
            assert_eq!(
                lib.matches(marker).count(),
                1,
                "lib.rs must contain `{marker}` exactly once, live"
            );
            lib.find(marker)
                .expect("the exactly-once check above guarantees one match")
        })
        .collect();
    assert!(
        positions[0] < positions[1] && positions[1] < positions[2],
        "the data root must be managed after the layout and before the \
         database open (T-024); marker positions: {positions:?}"
    );
}

#[test]
fn setup_continues_into_recovery_mode_on_a_corrupt_database() {
    // The lifecycle pin: the corrupt branch must log through the facade
    // and return Ok (recovery mode), while every other init failure
    // keeps the abort (`return Err`).
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    let markers = [
        "Err(error @ db::DbInitError::Corrupt { .. })",
        "logging::db_corrupt(&app_error);",
        "return Ok(());",
    ];
    let positions: Vec<usize> = markers
        .iter()
        .map(|marker| {
            assert_eq!(
                lib.matches(marker).count(),
                1,
                "lib.rs must contain `{marker}` exactly once, live"
            );
            lib.find(marker)
                .expect("the exactly-once check above guarantees one match")
        })
        .collect();
    for pair in positions.windows(2) {
        assert!(
            pair[0] < pair[1],
            "the corrupt verdict must be logged and the hook must return \
             Ok after it (recovery mode, T-024); marker positions: {positions:?}"
        );
    }
    // And the abort half stays: the other init failures return Err.
    assert_eq!(
        lib.matches("return Err(error.into());").count(),
        1,
        "the non-corrupt init failures must keep their D-027 abort"
    );
}

#[test]
fn the_commands_are_registered_with_the_specta_builder() {
    // The §7.10 names, registered exactly once each — D-009's list,
    // spelled from the spec, never derived from the code.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    for name in [
        "app_open_data_folder",
        "db_backups_list",
        "db_restore_backup",
        "db_recreate",
    ] {
        assert_eq!(
            lib.matches(&format!("commands::{name}")).count(),
            1,
            "lib.rs must register the `{name}` command exactly once (§7.10, D-009)"
        );
    }
}

#[test]
fn the_data_folder_opens_through_the_opener_plugin_from_rust() {
    // SEC-001: the opener call lives in Rust, behind the app command —
    // the webview has no `opener:` permission (pinned separately by
    // the capabilities pin in config_tests.rs). This pin keeps the
    // command wired to the plugin's Rust API.
    let commands = crate::config_tests::strip_comments(include_str!("commands.rs"));
    assert_eq!(
        commands.matches(".opener()").count(),
        1,
        "commands.rs must open the data folder through the opener plugin's \
         Rust API exactly once (SEC-001)"
    );
    assert!(
        commands.contains("open_path("),
        "the opener call must be `open_path` (a folder in the file manager)"
    );
    // The empty-capability contract this command must not disturb.
    let capabilities = include_str!("../capabilities/default.json");
    assert!(
        !capabilities.contains("opener:"),
        "capabilities/default.json must grant no opener permission (SEC-001)"
    );
}

/// Sets a file's modification time to a fixed UTC instant (the mtime
/// is the listing's ordering key; the file system's write-time
/// granularity must not decide the expected order).
fn set_mtime(path: &Path, instant: &str) {
    let timestamp: jiff::Timestamp = instant.parse().expect("the fixed mtime must parse");
    let system_time = std::time::SystemTime::try_from(timestamp)
        .expect("the fixed mtime must convert to SystemTime");
    let file = std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap_or_else(|error| panic!("opening {} for utime must work: {error}", path.display()));
    file.set_modified(system_time).unwrap_or_else(|error| {
        panic!("setting the mtime of {} must work: {error}", path.display())
    });
}
