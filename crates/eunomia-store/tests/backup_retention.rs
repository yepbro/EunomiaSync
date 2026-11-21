//! Behavior tests for the backup retention service (spec §7.6
//! BKP-003/BKP-006, §6.4 "Backup: file deleted by limit", §6.5
//! `backup.retention`, §9.4 phase 4 step 2; task T-114).
//!
//! The tests run against a real database opened through `open_db`
//! and migrated with `run_migrations` (the `tests/library_versions.
//! rs` pattern) and a real data directory in a temporary folder, and
//! drive the service through its public API — [`apply`] — reading
//! the `backup_records` table and the backups tree back directly:
//! the table and the files ARE the specified artifacts (§6.2/§5.5),
//! so their state is the observable behavior.
//!
//! The `backup_records` writer service does not exist yet (the
//! pipeline task lands it), so the fixtures seed rows with plain
//! SQL — the same "the table is the artifact" stance the
//! `library_versions` tests take — with the exact shapes that
//! writer will produce: §5.5 relative layout paths built through
//! the REAL core builders (`backup_relative_path` — the 19-char
//! BKP-001 stamp and the cleaned slug, pinning the store's
//! deletion guard against the builder's spelling), DB-002
//! `created_at` stamps, `ok` for every checked copy (BKP-010: a
//! record exists only after its check passed) and the terminal
//! statuses as history.
//!
//! Pinned behavior:
//!
//! * the BKP-003 acceptance criterion as a process — five syncs,
//!   each followed by the phase-4 retention at N=3, leaving after
//!   every run exactly the newest `min(n, 3)` copies alive and
//!   after the fifth the three newest (files alive, records `ok`),
//!   the older two deleted and marked `pruned`;
//! * keep 1, and the zero/negative clamp: the newest copy of a
//!   pair is never deleted;
//! * a same-millisecond pair resolves "newest" by the id (UUIDv7
//!   TEXT order, D-031);
//! * the limit is per "project and file" pair — two projects, two
//!   files in one project, each pair prunes alone;
//! * terminal-status records (`pruned`, `missing`, `discarded`)
//!   are invisible to the limit: no keep slot consumed, no row
//!   rewritten;
//! * a prune-window record whose file already vanished converges
//!   (`pruned`, no failure), and a second run over the pruned
//!   state changes nothing;
//! * a removal the filesystem refuses: the record is skipped and
//!   reported (never marked `pruned` over a live file), the other
//!   records still prune, and the obstacle removed, the next run
//!   converges;
//! * a stored `backup_path` outside the `backups/` layout is
//!   reported and never deleted;
//! * a failed mark aborts the run with the caller's rollback
//!   leaving every status untouched (the files already removed stay
//!   removed — the filesystem has no rollback), and the next run
//!   converges;
//! * an empty table and pairs within the limit are no-ops.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eunomia_core::backup_name::{BackupTime, backup_relative_path};
use eunomia_core::project::{BaseName as CoreBaseName, ProjectId, Scheme as CoreScheme};
use eunomia_store::backup_retention::{self, PruneFailure};
use eunomia_store::projects::{AddedVia, NewProject, create as create_project};
use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};
use eunomia_store::storage::{StorageError, open_db, run_migrations};
use eunomia_store::values::new_id;
use rusqlite::Connection;

/// A temporary directory removed when the guard is dropped (the
/// `tests/library_versions.rs` pattern).
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Creates a unique temporary directory holding a migrated database
/// (opened through `open_db`, so the DB-001 pragma set applies) and
/// an empty data directory whose `backups/` root exists — the layout
/// the §5.5 copies live under. Returns the connection and the data
/// directory path.
fn retention_db(label: &str) -> (Connection, TempDir, PathBuf) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be past the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "eunomia-store-backup-retention-{label}-{}-{nanos}",
        std::process::id(),
    ));
    std::fs::create_dir_all(dir.join("db")).expect("temp db directory should be creatable");
    std::fs::create_dir_all(dir.join("data").join("backups"))
        .expect("temp data backups directory should be creatable");
    let mut conn =
        open_db(&dir.join("db").join("eunomiasync.sqlite3")).expect("open_db should succeed");
    run_migrations(&mut conn, Path::new("unused-backup-dir"))
        .expect("migrations should apply to a fresh database");
    let data_dir = dir.join("data");
    (conn, TempDir(dir), data_dir)
}

/// Registers project `n` and returns its id.
fn seed_project(conn: &Connection, n: usize) -> String {
    let project = create_project(
        conn,
        &NewProject {
            path: &format!("/repos/repo-{n}"),
            path_key: &format!("/repos/repo-{n}"),
            name: &format!("Project {n}"),
            label: &format!("P{n}"),
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
    .unwrap_or_else(|err| panic!("seed project {n} should create, got {err}"));
    project.id
}

/// The DB-002 `created_at` stamp of sync `n` (n = 1…9): one second
/// apart, ascending — the fixture's timeline, matching the civil
/// fields [`fixture_time`] spells into the copy names.
fn stamp(n: usize) -> String {
    format!("2026-11-21T10:00:{n:02}.000Z")
}

/// The [`BackupTime`] of sync `n` — the same instant [`stamp`]
/// renders as a cell, in the builder's civil-field form.
fn fixture_time(n: usize) -> BackupTime {
    BackupTime {
        year: 2026,
        month: 11,
        day: 21,
        hour: 10,
        minute: 0,
        second: n as u8,
        millisecond: 0,
    }
}

/// The full 64-hex digest of copy `n` — distinct per `n` so the
/// builders' `hash8` name component differs too.
fn digest(n: usize) -> String {
    format!("{n:02x}").repeat(32)
}

/// The §5.5 layout path of copy `n`, built through the REAL core
/// builders — the same `backup_relative_path` the fs layer and the
/// future record writer spell with — so the fixtures seed exactly
/// the shape that writer will produce (the 19-char BKP-001 stamp,
/// the cleaned slug) and the store's deletion guard is pinned
/// against the builder's spelling, not a hand-copied one.
fn layout_path(project_id: &str, project_name: &str, n: usize, file: BaseName) -> String {
    backup_relative_path(
        &ProjectId::new(project_id.to_owned()),
        project_name,
        CoreScheme::A,
        core_base_name(file),
        fixture_time(n),
        &digest(n),
    )
    .expect("the fixture's builder inputs are valid by construction")
}

/// Maps the store-side base-name vocabulary onto core's builders'
/// enum — the fixture seeds projects through the store's types and
/// copy names through core's.
fn core_base_name(file: BaseName) -> CoreBaseName {
    match file {
        BaseName::AgentsMd => CoreBaseName::AgentsMd,
        BaseName::ClaudeMd => CoreBaseName::ClaudeMd,
    }
}

/// Inserts one `backup_records` row — the shape the pipeline's
/// writer will produce: `ok` for a checked copy, the DB-002 stamp,
/// a full 64-hex digest, size in bytes, the builder-spelled layout
/// path. `id` is explicit: ids and layout paths are table-unique
/// (PRIMARY KEY / UNIQUE), so multi-pair fixtures must name their
/// rows and files.
#[allow(clippy::too_many_arguments)]
fn seed_record(
    conn: &Connection,
    id: &str,
    n: usize,
    project_id: &str,
    project_name: &str,
    source_path: &str,
    file: BaseName,
    status: &str,
) -> String {
    let backup_path = layout_path(project_id, project_name, n, file);
    seed_record_with_path(
        conn,
        id,
        project_id,
        source_path,
        &backup_path,
        status,
        &stamp(n),
        &digest(n),
    );
    backup_path
}

/// Inserts one `backup_records` row with an explicit id,
/// `backup_path` and digest — the escape hatch for hostile-path
/// fixtures.
#[allow(clippy::too_many_arguments)]
fn seed_record_with_path(
    conn: &Connection,
    id: &str,
    project_id: &str,
    source_path: &str,
    backup_path: &str,
    status: &str,
    created_at: &str,
    sha256: &str,
) {
    conn.execute(
        "INSERT INTO backup_records (
             id, project_id, sync_run_id, source_path, backup_path,
             size_bytes, sha256, status, created_at
         ) VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            id,
            project_id,
            source_path,
            backup_path,
            (backup_path.len() as i64),
            sha256,
            status,
            created_at,
        ],
    )
    .unwrap_or_else(|err| panic!("seed record {id} should insert, got {err}"));
}

/// Writes a real copy file at `data_dir/<backup_path>` with
/// copy-distinct content.
fn write_copy(data_dir: &Path, backup_path: &str, content: &str) {
    let path = data_dir.join(backup_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the copy's folder should be creatable");
    }
    std::fs::write(&path, content).expect("the copy file should be writable");
}

/// One pair's rows as `(created_at, status, backup_path)`, in
/// timeline order — the table read straight back.
fn rows_of(
    conn: &Connection,
    project_id: &str,
    source_path: &str,
) -> Vec<(String, String, String)> {
    let mut statement = conn
        .prepare(
            "SELECT created_at, status, backup_path FROM backup_records
              WHERE project_id = ?1 AND source_path = ?2
              ORDER BY created_at",
        )
        .expect("the rows query should prepare");
    statement
        .query_map(rusqlite::params![project_id, source_path], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .expect("the rows query should run")
        .collect::<Result<Vec<_>, _>>()
        .expect("the rows should decode")
}

/// Every `.bak` file under the data directory's `backups` tree, as
/// §5.5 layout strings, sorted — the disk state read straight back.
fn copies_on_disk(data_dir: &Path) -> Vec<String> {
    let mut copies = Vec::new();
    let backups_root = data_dir.join("backups");
    let mut stack = vec![backups_root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("the backups tree should be listable") {
            let entry = entry.expect("a backups entry should be readable");
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let relative = path
                    .strip_prefix(data_dir)
                    .expect("the copy lives under the data directory");
                copies.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    copies.sort();
    copies
}

/// Seeds one sync's copy of the pair — the record and the file,
/// stamps and digest growing with `n` — and returns the layout
/// path.
fn seed_sync(
    conn: &Connection,
    data_dir: &Path,
    project_id: &str,
    project_name: &str,
    n: usize,
) -> String {
    let path = seed_record(
        conn,
        &format!("rec-{n:02}"),
        n,
        project_id,
        project_name,
        "/repos/repo-1/AGENTS.md",
        BaseName::AgentsMd,
        "ok",
    );
    write_copy(data_dir, &path, &format!("copy {n}"));
    path
}

/// Seeds five syncs' copies of the pair WITHOUT applying retention
/// — the fixture for tests that examine one application over a
/// grown history. Returns the five layout paths, oldest first.
fn five_seeded(
    conn: &Connection,
    data_dir: &Path,
    project_id: &str,
    project_name: &str,
) -> Vec<String> {
    (1..=5)
        .map(|n| seed_sync(conn, data_dir, project_id, project_name, n))
        .collect()
}

/// The BKP-003 acceptance criterion, verbatim and as a process:
/// five syncs, each followed by the phase-4 retention at N=3 —
/// after every run the newest `min(n, 3)` copies are alive, and
/// after the fifth exactly the three newest remain: their files
/// alive and their records `ok`, the two oldest files deleted and
/// their records `pruned` (§6.4).
#[test]
fn keep_3_after_five_syncs_leaves_exactly_the_three_newest() {
    let (conn, _guard, data_dir) = retention_db("n3-five-syncs");
    let project_id = seed_project(&conn, 1);

    let mut paths = Vec::new();
    for n in 1..=5 {
        paths.push(seed_sync(&conn, &data_dir, &project_id, "Project 1", n));
        let outcome = backup_retention::apply(&conn, &data_dir, 3)
            .expect("the per-sync retention should succeed");
        // The run never prunes below the newest three, and prunes
        // exactly the one copy the new sync pushed past the window.
        let expected_pruned = u64::from(n > 3);
        assert_eq!(
            outcome.pruned, expected_pruned,
            "sync {n}: the per-run prune count"
        );
        assert!(outcome.failures.is_empty(), "sync {n}: nothing fails");
        let window_start = paths.len().saturating_sub(3);
        assert_eq!(
            copies_on_disk(&data_dir),
            paths[window_start..],
            "sync {n}: exactly the newest min(n, 3) files alive"
        );
    }

    assert_eq!(
        rows_of(&conn, &project_id, "/repos/repo-1/AGENTS.md")
            .iter()
            .map(|row| row.1.as_str())
            .collect::<Vec<_>>(),
        vec!["pruned", "pruned", "ok", "ok", "ok"],
        "after five syncs: the two oldest records pruned, the three newest kept"
    );
    assert_eq!(
        copies_on_disk(&data_dir),
        &paths[2..],
        "exactly the three newest files must survive"
    );
}

/// Keep 1 collapses a pair to its newest copy.
#[test]
fn keep_1_leaves_only_the_newest_copy() {
    let (conn, _guard, data_dir) = retention_db("n1");
    let project_id = seed_project(&conn, 1);
    let paths = five_seeded(&conn, &data_dir, &project_id, "Project 1");

    let outcome =
        backup_retention::apply(&conn, &data_dir, 1).expect("retention at keep 1 should succeed");

    assert_eq!(outcome.pruned, 4, "the four older copies must be pruned");
    assert!(outcome.failures.is_empty(), "nothing should fail");
    assert_eq!(
        copies_on_disk(&data_dir),
        vec![paths[4].clone()],
        "only the newest file must survive"
    );
    let rows = rows_of(&conn, &project_id, "/repos/repo-1/AGENTS.md");
    let statuses: Vec<&str> = rows.iter().map(|row| row.1.as_str()).collect();
    assert_eq!(statuses, vec!["pruned", "pruned", "pruned", "pruned", "ok"]);
}

/// A zero or negative `keep` is a caller bug (§6.5 validates
/// 1–200), not a user setting: the service clamps it to 1 rather
/// than wiping a pair's whole backup history — the newest copy is
/// never deleted (the ALG-072 clamp precedent, D-102).
#[test]
fn zero_and_negative_keep_still_keep_the_newest_copy() {
    let (conn, _guard, data_dir) = retention_db("clamp");
    let project_id = seed_project(&conn, 1);
    let paths = five_seeded(&conn, &data_dir, &project_id, "Project 1");

    let outcome =
        backup_retention::apply(&conn, &data_dir, 0).expect("keep 0 must be clamped, not rejected");
    assert_eq!(outcome.pruned, 4, "keep 0 must behave as keep 1");
    assert!(outcome.failures.is_empty(), "keep 0: nothing fails");

    // The clamped negative limit over the already-pruned state is a
    // no-op that still refuses to touch the newest copy.
    let outcome = backup_retention::apply(&conn, &data_dir, -7)
        .expect("a negative keep must be clamped, not rejected");
    assert_eq!(outcome.pruned, 0, "keep -7 must behave as keep 1");
    assert!(outcome.failures.is_empty(), "keep -7: nothing fails");

    assert_eq!(
        copies_on_disk(&data_dir),
        vec![paths[4].clone()],
        "the newest copy must survive every clamped run"
    );
    let rows = rows_of(&conn, &project_id, "/repos/repo-1/AGENTS.md");
    let statuses: Vec<&str> = rows.iter().map(|row| row.1.as_str()).collect();
    assert_eq!(statuses, vec!["pruned", "pruned", "pruned", "pruned", "ok"]);
}

/// A same-millisecond pair — two records sharing `created_at` to
/// the DB-002 millisecond — resolves "newest" by the id: UUIDv7
/// TEXT sorts by creation (D-031), so the later-generated id is
/// the copy that survives.
#[test]
fn a_same_millisecond_pair_resolves_by_the_newest_id() {
    let (conn, _guard, data_dir) = retention_db("tie-break");
    let project_id = seed_project(&conn, 1);
    let source = "/repos/repo-1/AGENTS.md";
    // Two real UUIDv7 ids from the store's own generator: strictly
    // increasing within one process (D-031), so `older` predates
    // `newer` by construction.
    let older_id = new_id();
    let newer_id = new_id();
    assert!(
        older_id < newer_id,
        "new_id must be strictly increasing in one process (D-031)"
    );
    // Same stamp, distinct digests (else the builders would spell
    // one name for both rows against the UNIQUE column).
    let older_digest = "ab".repeat(32);
    let newer_digest = "cd".repeat(32);
    let older_path = backup_relative_path(
        &ProjectId::new(project_id.clone()),
        "Project 1",
        CoreScheme::A,
        CoreBaseName::AgentsMd,
        fixture_time(3),
        &older_digest,
    )
    .expect("the fixture's builder inputs are valid by construction");
    let newer_path = backup_relative_path(
        &ProjectId::new(project_id.clone()),
        "Project 1",
        CoreScheme::A,
        CoreBaseName::AgentsMd,
        fixture_time(3),
        &newer_digest,
    )
    .expect("the fixture's builder inputs are valid by construction");
    seed_record_with_path(
        &conn,
        &older_id,
        &project_id,
        source,
        &older_path,
        "ok",
        &stamp(3),
        &older_digest,
    );
    seed_record_with_path(
        &conn,
        &newer_id,
        &project_id,
        source,
        &newer_path,
        "ok",
        &stamp(3),
        &newer_digest,
    );
    write_copy(&data_dir, &older_path, "older copy");
    write_copy(&data_dir, &newer_path, "newer copy");

    let outcome =
        backup_retention::apply(&conn, &data_dir, 1).expect("retention over a tie should succeed");

    assert_eq!(outcome.pruned, 1, "the older id is the one past the window");
    assert!(outcome.failures.is_empty(), "nothing should fail");
    let status_of = |id: &str| {
        conn.query_row(
            "SELECT status FROM backup_records WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get::<_, String>(0),
        )
        .expect("the seeded row must be readable")
    };
    assert_eq!(status_of(&older_id), "pruned", "the older id is pruned");
    assert_eq!(status_of(&newer_id), "ok", "the newer id keeps the slot");
    assert_eq!(copies_on_disk(&data_dir), vec![newer_path]);
}

/// The limit is per "project and file" pair (BKP-003): three pairs
/// — two projects with the same file name, and two files in one
/// project — each prunes against its own window, never another
/// pair's.
#[test]
fn the_limit_is_per_project_and_file_pair() {
    let (conn, _guard, data_dir) = retention_db("pairs");
    let project_a = seed_project(&conn, 1);
    let project_b = seed_project(&conn, 2);
    // Pair 1: project A, AGENTS.md — five copies, the only pair over
    // the limit.
    let pair1 = five_seeded(&conn, &data_dir, &project_a, "Project 1");
    // Pair 2: project A, CLAUDE.md — two copies, same project.
    let mut pair2 = Vec::new();
    for n in 1..=2 {
        let path = seed_record(
            &conn,
            &format!("rec-claude-{n:02}"),
            n,
            &project_a,
            "Project 1",
            "/repos/repo-1/CLAUDE.md",
            BaseName::ClaudeMd,
            "ok",
        );
        write_copy(&data_dir, &path, &format!("claude {n}"));
        pair2.push(path);
    }
    // Pair 3: project B, AGENTS.md — two copies, same file name as
    // pair 1, different project.
    let mut pair3 = Vec::new();
    for n in 1..=2 {
        let path = seed_record(
            &conn,
            &format!("rec-beta-{n:02}"),
            n,
            &project_b,
            "Project 2",
            "/repos/repo-2/AGENTS.md",
            BaseName::AgentsMd,
            "ok",
        );
        write_copy(&data_dir, &path, &format!("beta {n}"));
        pair3.push(path);
    }

    let outcome = backup_retention::apply(&conn, &data_dir, 3)
        .expect("retention over three pairs should succeed");

    assert_eq!(outcome.pruned, 2, "only pair 1's two oldest must go");
    assert!(outcome.failures.is_empty(), "nothing should fail");
    assert_eq!(
        rows_of(&conn, &project_a, "/repos/repo-1/AGENTS.md")
            .iter()
            .map(|row| row.1.as_str())
            .collect::<Vec<_>>(),
        vec!["pruned", "pruned", "ok", "ok", "ok"],
        "the over-limit pair prunes"
    );
    assert_eq!(
        rows_of(&conn, &project_a, "/repos/repo-1/CLAUDE.md")
            .iter()
            .map(|row| row.1.as_str())
            .collect::<Vec<_>>(),
        vec!["ok", "ok"],
        "the same project's other file is a different pair"
    );
    assert_eq!(
        rows_of(&conn, &project_b, "/repos/repo-2/AGENTS.md")
            .iter()
            .map(|row| row.1.as_str())
            .collect::<Vec<_>>(),
        vec!["ok", "ok"],
        "another project is a different pair"
    );
    let mut expected_disk = pair1[2..].to_vec();
    expected_disk.extend_from_slice(&pair2);
    expected_disk.extend_from_slice(&pair3);
    expected_disk.sort();
    assert_eq!(copies_on_disk(&data_dir), expected_disk);
}

/// Terminal-status records — `pruned`, `missing`, `discarded` —
/// are history, not copies (BKP-006: the retention set is the
/// checked ones): they consume no keep slot and are never
/// rewritten by the limit.
#[test]
fn terminal_status_records_are_invisible_to_the_limit() {
    let (conn, _guard, data_dir) = retention_db("terminal");
    let project_id = seed_project(&conn, 1);
    let source = "/repos/repo-1/AGENTS.md";
    // Timeline: t1 ok, t2 pruned, t3 ok, t4 missing, t5 ok, t6
    // discarded (the BKP-005 sweep's status). The terminal rows
    // have no file on disk (their copies are gone — that is what
    // those statuses mean).
    seed_record(
        &conn,
        "rec-01",
        1,
        &project_id,
        "Project 1",
        source,
        BaseName::AgentsMd,
        "ok",
    );
    seed_record(
        &conn,
        "rec-02",
        2,
        &project_id,
        "Project 1",
        source,
        BaseName::AgentsMd,
        "pruned",
    );
    seed_record(
        &conn,
        "rec-03",
        3,
        &project_id,
        "Project 1",
        source,
        BaseName::AgentsMd,
        "ok",
    );
    seed_record(
        &conn,
        "rec-04",
        4,
        &project_id,
        "Project 1",
        source,
        BaseName::AgentsMd,
        "missing",
    );
    seed_record(
        &conn,
        "rec-05",
        5,
        &project_id,
        "Project 1",
        source,
        BaseName::AgentsMd,
        "ok",
    );
    seed_record(
        &conn,
        "rec-06",
        6,
        &project_id,
        "Project 1",
        source,
        BaseName::AgentsMd,
        "discarded",
    );
    for n in [1, 3, 5] {
        write_copy(
            &data_dir,
            &layout_path(&project_id, "Project 1", n, BaseName::AgentsMd),
            &format!("copy {n}"),
        );
    }
    let before = rows_of(&conn, &project_id, source);

    // keep 2 over the ok rows {t5, t3}: t1 is the only prunable
    // record; t2, t4 and t6 are invisible.
    let outcome = backup_retention::apply(&conn, &data_dir, 2)
        .expect("retention with terminal rows should succeed");

    assert_eq!(
        outcome.pruned, 1,
        "only the ok row outside the window prunes"
    );
    assert!(outcome.failures.is_empty(), "nothing should fail");
    let after = rows_of(&conn, &project_id, source);
    assert_eq!(after[0].1, "pruned", "t1 (the ok outsider) is pruned");
    assert_eq!(
        after[1], before[1],
        "the pruned row must be untouched (no re-mark, no restamp)"
    );
    assert_eq!(after[3], before[3], "the missing row must be untouched");
    assert_eq!(after[5], before[5], "the discarded row must be untouched");
    assert_eq!(after[2].1, "ok", "t3 keeps its slot");
    assert_eq!(after[4].1, "ok", "t5 keeps its slot");
    assert_eq!(
        copies_on_disk(&data_dir),
        vec![
            layout_path(&project_id, "Project 1", 3, BaseName::AgentsMd),
            layout_path(&project_id, "Project 1", 5, BaseName::AgentsMd)
        ],
        "exactly the two kept copies' files survive"
    );
}

/// A prune-window record whose file already vanished — a crash
/// between a removal and its mark, or an external deletion —
/// converges: the removal's `NotFound` is the success it is, the
/// record is marked `pruned`, and a second run over the pruned
/// state is a complete no-op (idempotency).
#[test]
fn a_vanished_copy_converges_and_a_second_run_changes_nothing() {
    let (conn, _guard, data_dir) = retention_db("vanished");
    let project_id = seed_project(&conn, 1);
    let source = "/repos/repo-1/AGENTS.md";
    for n in 1..=4 {
        seed_record(
            &conn,
            &format!("rec-{n:02}"),
            n,
            &project_id,
            "Project 1",
            source,
            BaseName::AgentsMd,
            "ok",
        );
    }
    // Copies 1, 3 and 4 exist on disk; copy 2's file was lost (a
    // crash between a removal and its mark, or an external
    // deletion). Copy 1 is a normal prune, so one pass exercises
    // both the real removal and the NotFound convergence.
    write_copy(
        &data_dir,
        &layout_path(&project_id, "Project 1", 1, BaseName::AgentsMd),
        "copy 1",
    );
    write_copy(
        &data_dir,
        &layout_path(&project_id, "Project 1", 3, BaseName::AgentsMd),
        "copy 3",
    );
    write_copy(
        &data_dir,
        &layout_path(&project_id, "Project 1", 4, BaseName::AgentsMd),
        "copy 4",
    );

    let outcome = backup_retention::apply(&conn, &data_dir, 2)
        .expect("retention over a vanished copy should succeed");

    assert_eq!(
        outcome.pruned, 2,
        "the vanished copy converges instead of failing"
    );
    assert!(
        outcome.failures.is_empty(),
        "the vanished copy is not a failure"
    );
    assert_eq!(
        rows_of(&conn, &project_id, source)
            .iter()
            .map(|row| row.1.as_str())
            .collect::<Vec<_>>(),
        vec!["pruned", "pruned", "ok", "ok"]
    );

    let before = rows_of(&conn, &project_id, source);
    let disk_before = copies_on_disk(&data_dir);
    let outcome =
        backup_retention::apply(&conn, &data_dir, 2).expect("the second run should succeed");
    assert_eq!(
        outcome.pruned, 0,
        "a run over a pruned state prunes nothing"
    );
    assert!(
        outcome.failures.is_empty(),
        "a run over a pruned state reports nothing"
    );
    assert_eq!(rows_of(&conn, &project_id, source), before);
    assert_eq!(copies_on_disk(&data_dir), disk_before);
}

/// A removal the filesystem refuses — here a directory squatting
/// on the copy's name, so `remove_file` cannot succeed — must not
/// mark the record `pruned` over a live path: the record is
/// skipped and reported, the OTHER prunable records of the same
/// run still prune, and once the obstacle is gone the next run
/// converges.
#[test]
fn a_refused_removal_skips_the_record_reports_it_and_continues() {
    let (conn, _guard, data_dir) = retention_db("refused");
    let project_id = seed_project(&conn, 1);
    let source = "/repos/repo-1/AGENTS.md";
    for n in 1..=3 {
        let path = seed_record(
            &conn,
            &format!("rec-{n:02}"),
            n,
            &project_id,
            "Project 1",
            source,
            BaseName::AgentsMd,
            "ok",
        );
        if n != 1 {
            write_copy(&data_dir, &path, &format!("copy {n}"));
        }
    }
    // Copy 1's name is held by a directory: `remove_file` on it
    // fails on every platform (EISDIR on Unix, access denied on
    // Windows) — the obstacle outlives the §9.5 retries.
    let obstacle = data_dir.join(layout_path(&project_id, "Project 1", 1, BaseName::AgentsMd));
    std::fs::create_dir_all(&obstacle).expect("the obstacle directory should be creatable");

    let outcome = backup_retention::apply(&conn, &data_dir, 1)
        .expect("a refused removal is a report, not an error");

    assert_eq!(outcome.pruned, 1, "the unobstructed record still prunes");
    assert_eq!(
        outcome.failures.len(),
        1,
        "the obstructed record is reported"
    );
    assert_eq!(
        outcome.failures[0].record_id, "rec-01",
        "the report names the skipped record"
    );
    assert_eq!(
        outcome.failures[0].backup_path,
        layout_path(&project_id, "Project 1", 1, BaseName::AgentsMd)
    );
    assert!(
        matches!(outcome.failures[0].reason, PruneFailure::RemoveFailed(_)),
        "the reason is the refused removal, got {:?}",
        outcome.failures[0].reason
    );
    assert_eq!(
        rows_of(&conn, &project_id, source)
            .iter()
            .map(|row| row.1.as_str())
            .collect::<Vec<_>>(),
        vec!["ok", "pruned", "ok"],
        "the obstructed record keeps its ok status for the next run"
    );
    assert!(
        obstacle.is_dir(),
        "the obstacle itself must not have been touched"
    );
    assert_eq!(
        copies_on_disk(&data_dir),
        vec![layout_path(&project_id, "Project 1", 3, BaseName::AgentsMd)],
        "the newest copy is kept, the middle one pruned"
    );

    // The obstacle gone, the next run converges the skipped record.
    std::fs::remove_dir_all(&obstacle).expect("the obstacle should be removable");
    write_copy(
        &data_dir,
        &layout_path(&project_id, "Project 1", 1, BaseName::AgentsMd),
        "copy 1 restored",
    );
    let outcome =
        backup_retention::apply(&conn, &data_dir, 1).expect("the retry run should succeed");
    assert_eq!(outcome.pruned, 1, "the retried record now prunes");
    assert!(
        outcome.failures.is_empty(),
        "the retry has nothing to report"
    );
    assert_eq!(
        rows_of(&conn, &project_id, source)
            .iter()
            .map(|row| row.1.as_str())
            .collect::<Vec<_>>(),
        vec!["pruned", "pruned", "ok"]
    );
}

/// A stored `backup_path` outside the §5.5 layout — a `..` escape
/// and an absolute path — is reported and its target NEVER touched:
/// the deletion guard refuses the cell before any filesystem call.
#[test]
fn hostile_backup_paths_are_reported_and_never_deleted() {
    let (conn, _guard, data_dir) = retention_db("hostile");
    let project_id = seed_project(&conn, 1);
    let source = "/repos/repo-1/AGENTS.md";
    // The escape's target, outside the data directory: it must be
    // exactly as untouched as the record that names it.
    let escaped = data_dir.parent().unwrap().join("escaped.bak");
    std::fs::write(&escaped, "do not delete").expect("the escaped file should be writable");
    seed_record_with_path(
        &conn,
        "rec-escape",
        &project_id,
        source,
        "../escaped.bak",
        "ok",
        &stamp(1),
        &"1".repeat(64),
    );
    seed_record_with_path(
        &conn,
        "rec-absolute",
        &project_id,
        source,
        "/etc/absolute.bak",
        "ok",
        &stamp(2),
        &"2".repeat(64),
    );
    let newest = seed_record(
        &conn,
        "rec-03",
        3,
        &project_id,
        "Project 1",
        source,
        BaseName::AgentsMd,
        "ok",
    );
    write_copy(&data_dir, &newest, "copy 3");

    let outcome = backup_retention::apply(&conn, &data_dir, 1)
        .expect("hostile cells are reports, not errors");

    assert_eq!(outcome.pruned, 0, "nothing prunes");
    assert_eq!(outcome.failures.len(), 2, "both hostile cells are reported");
    let mut reported: Vec<&str> = outcome
        .failures
        .iter()
        .map(|failure| failure.record_id.as_str())
        .collect();
    reported.sort_unstable();
    assert_eq!(reported, vec!["rec-absolute", "rec-escape"]);
    for failure in &outcome.failures {
        assert!(
            matches!(failure.reason, PruneFailure::Path(_)),
            "{} must be a path rejection, got {:?}",
            failure.record_id,
            failure.reason
        );
    }
    assert_eq!(
        std::fs::read(&escaped).expect("the escaped file must still exist"),
        b"do not delete",
        "the guard must never delete outside the backups tree"
    );
    assert!(
        rows_of(&conn, &project_id, source)
            .iter()
            .all(|row| row.1 == "ok"),
        "the hostile records keep their ok status for a human"
    );
    assert_eq!(copies_on_disk(&data_dir), vec![newest]);
}

/// A mark that fails mid-run — injected as a trigger aborting the
/// second record's UPDATE — surfaces as the SQL error it is, and
/// the caller's rollback leaves every status untouched while the
/// files already removed stay removed (the filesystem has no
/// rollback; delete-first is what makes that survivable). The next
/// run converges: the vanished files' records prune on `NotFound`.
#[test]
fn a_failed_mark_rolls_back_and_the_next_run_converges() {
    let (conn, _guard, data_dir) = retention_db("rollback");
    let project_id = seed_project(&conn, 1);
    let source = "/repos/repo-1/AGENTS.md";
    for n in 1..=4 {
        let path = seed_record(
            &conn,
            &format!("rec-{n:02}"),
            n,
            &project_id,
            "Project 1",
            source,
            BaseName::AgentsMd,
            "ok",
        );
        write_copy(&data_dir, &path, &format!("copy {n}"));
    }

    conn.execute_batch("BEGIN")
        .expect("the caller's transaction should open");
    conn.execute_batch(
        "CREATE TRIGGER refuse_second_prune
         BEFORE UPDATE ON backup_records
         WHEN NEW.id = 'rec-02'
         BEGIN SELECT RAISE(ABORT, 'injected mark failure'); END;",
    )
    .expect("the failing trigger should install");

    let err = backup_retention::apply(&conn, &data_dir, 1)
        .expect_err("the injected mark failure must abort the run");
    assert!(
        matches!(err, StorageError::Sql(_)),
        "the failure surfaces as the SQL error it is, got {err}"
    );

    conn.execute_batch("ROLLBACK")
        .expect("the caller's rollback should run");

    assert_eq!(
        rows_of(&conn, &project_id, source)
            .iter()
            .map(|row| row.1.as_str())
            .collect::<Vec<_>>(),
        vec!["ok", "ok", "ok", "ok"],
        "the rollback must undo every mark of the run — including rec-01's"
    );
    assert_eq!(
        copies_on_disk(&data_dir),
        vec![
            layout_path(&project_id, "Project 1", 3, BaseName::AgentsMd),
            layout_path(&project_id, "Project 1", 4, BaseName::AgentsMd)
        ],
        "the files removed before the failure stay removed (fs has no rollback)"
    );

    let outcome =
        backup_retention::apply(&conn, &data_dir, 1).expect("the convergence run should succeed");
    assert_eq!(
        outcome.pruned, 3,
        "the vanished files' records converge on NotFound, the live one prunes"
    );
    assert!(
        outcome.failures.is_empty(),
        "the convergence run has nothing to report"
    );
    assert_eq!(
        rows_of(&conn, &project_id, source)
            .iter()
            .map(|row| row.1.as_str())
            .collect::<Vec<_>>(),
        vec!["pruned", "pruned", "pruned", "ok"]
    );
    assert_eq!(
        copies_on_disk(&data_dir),
        vec![layout_path(&project_id, "Project 1", 4, BaseName::AgentsMd)]
    );
}

/// An empty table is a clean no-op, and a pair already within the
/// limit prunes and deletes nothing.
#[test]
fn an_empty_table_and_pairs_within_the_limit_change_nothing() {
    let (conn, _guard, data_dir) = retention_db("noops");
    let outcome = backup_retention::apply(&conn, &data_dir, 3)
        .expect("retention over an empty table should succeed");
    assert_eq!(outcome.pruned, 0, "an empty table prunes nothing");
    assert!(
        outcome.failures.is_empty(),
        "an empty table reports nothing"
    );

    let project_id = seed_project(&conn, 1);
    let source = "/repos/repo-1/AGENTS.md";
    let mut paths = Vec::new();
    for n in 1..=3 {
        let path = seed_record(
            &conn,
            &format!("rec-{n:02}"),
            n,
            &project_id,
            "Project 1",
            source,
            BaseName::AgentsMd,
            "ok",
        );
        write_copy(&data_dir, &path, &format!("copy {n}"));
        paths.push(path);
    }

    let outcome = backup_retention::apply(&conn, &data_dir, 3)
        .expect("retention within the limit should succeed");
    assert_eq!(outcome.pruned, 0, "a pair within the limit prunes nothing");
    assert!(
        outcome.failures.is_empty(),
        "a pair within the limit reports nothing"
    );
    assert_eq!(copies_on_disk(&data_dir), paths);
    assert_eq!(
        rows_of(&conn, &project_id, source)
            .iter()
            .map(|row| row.1.as_str())
            .collect::<Vec<_>>(),
        vec!["ok", "ok", "ok"]
    );
}
