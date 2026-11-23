//! Backup retention: keeping the newest N copies per "project and
//! file" pair (spec §7.6 BKP-003/BKP-006, §6.4 "Backup: file deleted
//! by limit", §9.4 phase 4 step 2).
//!
//! One entry point, [`apply`], for the sync pipeline to call after
//! a run finishes (§9.4 phase 4 — the caller landed with the
//! pipeline task: the phase-4 finish in src-tauri's `sync_phase4`
//! invokes it inside the run's finish transaction, and the integration
//! tests drive the service too): for every "project and file" pair —
//! the `(project_id, source_path)` of the `backup_records` rows —
//! the newest `keep` checked copies stay and every older one is
//! physically deleted from the backups folder, its record marked
//! `pruned` (§6.4: "file deleted by limit → the record gets the
//! `pruned` status").
//!
//! # The limit is a parameter, not a settings read
//!
//! The `LogLimits` precedent of `action_log::cleanup` and the
//! ALG-072 ruling of DECISIONS.md D-102: §6.5's `backup.retention`
//! range (1–200) has one owner — the settings repository (D-039) —
//! and the command layer reads the setting and passes the value on.
//! `keep` is therefore an `i64` parameter, and an out-of-range value
//! (0 or negative — a caller bug, not a user setting) is clamped to
//! 1 inside [`apply`]: the newest copy of a pair is never deleted,
//! the same unconditional guard D-102 applies to revision pruning.
//! The clamp keeps a buggy zero from silently wiping a project's
//! whole backup history.
//!
//! # Only checked copies are the retention's set (BKP-006)
//!
//! BKP-003/006 prune "only if the new copies are checked". The
//! schema has no `unchecked` status: §9.4 phase 2 inserts a record
//! only after the BKP-010 size + SHA-256 check passed, so every
//! record is a checked copy at insert, and the other three statuses
//! mark copies that no longer exist (`pruned`, `missing`,
//! `discarded`). The retention set is therefore exactly the
//! `status = 'ok'` rows:
//!
//! * a terminal-status row never consumes a keep slot — it is not a
//!   copy, it is the history of one;
//! * a terminal-status row is never rewritten — re-pruning a
//!   `pruned` row is a no-op by construction, and `missing`/
//!   `discarded` facts belong to their producers (§10's missing
//!   detection, BKP-005's discarded sweep), not to the limit.
//!
//! The run-level half of BKP-006 — retention runs only after every
//! new copy of the run was checked — is the pipeline's calling
//! contract: phase 4 is reached only when phase 2 verified all
//! copies; an `E_BACKUP_FAILED` abort ends the run before this
//! service is called. The record-level half is enforced here by the
//! `status = 'ok'` filter.
//!
//! # Ordering: which N are "the newest"
//!
//! `created_at` decides, descending — the DB-002 TEXT shape makes
//! the lexicographic order the chronological one, the same property
//! `action_log`'s time-ordered queries stand on. The `id` is the
//! tiebreaker (`created_at DESC, id DESC`): UUIDv7 TEXT sorts by
//! creation (D-031), so a same-millisecond pair resolves by true
//! creation order, and the order is deterministic regardless.
//! The pair key is `source_path` VERBATIM — the one-stable-spelling
//! contract of D-122, kept by the writer that landed with the
//! sync-phase-2 task (T-115): src-tauri's `sync_phase2` inserts
//! every record through `backup_records::insert` with `source_path`
//! set to the §9.4 phase 1 resolved target path's text (the pass
//! fact's `source_text`, on both insert paths — the success
//! transaction and the `E_BACKUP_FAILED` abort sweep). A writer
//! that drifts from the resolved spelling silently splits a file's
//! history into two windows, each capped separately (up to 2×N
//! copies, never an error) — the relative-`backup_path` ruling
//! below is the same contract for the other column.
//!
//! # Deletion precedes the mark; the mark is the caller's transaction
//!
//! §6.4 ties the status to the physical deletion, so [`apply`]
//! removes the file FIRST and marks the record only after the
//! removal succeeded — the self-healing order of the two:
//!
//! * a crash or rollback between the two leaves an `ok` record
//!   without a file; the next run's removal sees `NotFound`, which
//!   is convergence, not failure (the `remove_copy` ruling of
//!   D-091), and marks the record `pruned`. The mirror order would
//!   leave a `pruned` record with a live file — a record claiming
//!   history the restore screen must not offer. The convergence
//!   holds while the record still sits past the window: a limit
//!   raised before the next run leaves an `ok` record over a
//!   missing file — a state the §10 restore/`missing` detection is
//!   the designated closer for (no v1 producer of `missing`
//!   exists yet, so today it is a dormant, slot-consuming row);
//! * a removal that fails for a real reason (the file stays held
//!   after the §9.5 transient-hold retries, a permissions drift)
//!   does NOT mark the record: `pruned` asserts the file is gone,
//!   and a lying mark is worse than a deferred prune. The record
//!   stays `ok`, the failure is reported in
//!   [`RetentionOutcome::failures`] for the caller's log, and the
//!   next run retries it — the best-effort posture of
//!   `storage::daily_backup`'s retention, which likewise never
//!   fails its own run over a file it could not delete. The
//!   remaining records are still processed: one held file must not
//!   freeze the whole history behind the limit.
//! * the marks execute on the caller's connection and are atomic
//!   only inside the caller's transaction (D-037/D-051): a database
//!   failure mid-mark aborts with [`StorageError::Sql`], the
//!   caller's rollback undoes every mark of the run, and the files
//!   already removed stay removed — records `ok` without files,
//!   which the next run converges (idempotency, the paragraph
//!   above).
//!
//! # The deletion guard (SEC-006's spirit)
//!
//! `backup_path` is the §5.5 layout string — `backups/<project_id>__
//! <slug>/<file>.<UTC>.<hash8>.bak`, relative to the data directory
//! (D-091's `BackupCopy::relative_path` spelling) — and the service
//! joins it under the caller's `data_dir` before every removal. A
//! destructive operation does not trust a stored cell blindly: the
//! joined target must be relative, must start at the `backups`
//! root, must refuse `..` components (mid-path `.` cannot appear —
//! `Path::components` normalizes it away), and the name must be a
//! `.bak` file. A cell that fails any rule — corruption, a
//! hand-edited database — is reported as [`PruneFailure::Path`]
//! and NEVER deleted: the strict-name discipline of `storage`'s
//! daily-copy retention, applied to deletion. The guard judges the
//! CELL, not the filesystem: a symlink or junction planted inside a
//! writable data directory is outside its threat model (the cell is
//! machine-written, and the `backups` root is born `0700`, SEC-007
//! — an attacker with data-directory write access owns the copies
//! already).
//!
//! # What the service does not do
//!
//! No folder creation or sweeping of unreferenced crash debris
//! (D-091 leaves that to the run-level tasks), no `missing`
//! detection (§10 diagnostics), no action-log writes (the journal
//! is the caller's, D-051's boundary), no re-verification of the
//! surviving copies (BKP-010 checked them at creation; re-hashing
//! history on every sync is not in any BKP requirement), and no
//! row deletion: §6.4 keeps the pruned history as status, so the
//! table grows by one row per real copy and the window scans that
//! whole history each run — a volume §12.2 bounds, and an index
//! over `(status, project_id, source_path, created_at)` is a
//! schema task for the day it stops being true.
//!
//! # Testing plan
//!
//! Integration tests in `tests/backup_retention.rs` over a real
//! `open_db` + `run_migrations` database and real files in a
//! temporary data directory: the BKP-003 acceptance criterion as a
//! process (five syncs, each followed by the phase-4 retention at
//! N=3), keep 1, the zero/negative clamp, the same-millisecond
//! id tie-break, per-pair isolation (two projects, two files),
//! terminal statuses invisible to the limit (`pruned`, `missing`,
//! `discarded`), a prune-window record whose file already vanished
//! (convergence), the refused removal (skipped, reported, the rest
//! still pruned), the hostile `backup_path` (reported, never
//! touched), the transaction rollback on a failed mark with the
//! next run converging, and the empty-table / within-limit no-ops.
//! The fixtures' layout paths are built through the real core
//! builders, pinning the guard's accepted shape against the
//! builder's own spelling. Colocated unit tests pin the path
//! guard's vocabulary and the failure `Display` shapes.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use rusqlite::Connection;

use crate::storage::{StorageError, retry_transient_hold};

/// The `backups` root of the §5.5 layout — the one component a
/// prune target's path must start with (the deletion guard; see the
/// module docs). Kept here as the guard's own spelling, one module
/// away from `eunomia-fs`'s folder-creation constant on purpose:
/// the two crates do not depend on each other, and the integration
/// tests pin the accepted shape against the real builder output so
/// the spellings cannot drift silently.
const BACKUPS_ROOT: &str = "backups";

/// The backup file suffix of the §5.5 layout (`<file>.<UTC>.<hash8>
/// .bak`) — the deletion guard's name half.
const BACKUP_SUFFIX: &str = ".bak";

/// Applies the backup retention limit (BKP-003/BKP-006): for every
/// `(project_id, source_path)` pair, the newest `keep` records with
/// status `ok` stay; every older one's file is deleted from
/// `<data_dir>/<backup_path>` and its record is marked `pruned`
/// (§6.4). See the module docs for the full semantics — the clamp,
/// the checked-copy set, the delete-first order, the best-effort
/// failure posture.
///
/// `keep` arrives as a parameter (the `LogLimits`/D-102 precedent:
/// the command layer reads `backup.retention` and passes it); a
/// zero or negative value is clamped to 1, so the newest copy of a
/// pair is never deleted.
///
/// The statements run on the caller's connection: wrap the call in
/// the caller's transaction to make the marks atomic (D-037/D-051).
/// The file removals are filesystem effects a rollback cannot
/// undo — the delete-first order makes that safe (the module docs'
/// convergence argument).
///
/// # Errors
///
/// [`StorageError::Sql`] when the prunable-rows query or a mark
/// fails — the caller's rollback then undoes every mark of the run
/// (files already removed stay removed; the next run converges).
/// Filesystem failures are NOT errors: they are reported in
/// [`RetentionOutcome::failures`] and the affected records keep
/// their `ok` status for the next run to retry.
pub fn apply(
    conn: &Connection,
    data_dir: &Path,
    keep: i64,
) -> Result<RetentionOutcome, StorageError> {
    // The prunable rows: per pair, the `ok` records past the newest
    // `keep` ones. `keep` is clamped before the query (a zero or
    // negative value must not wipe a pair's whole history), and the
    // outer ORDER BY makes the walk — and therefore the failure
    // report — deterministic.
    let prunable = {
        let mut statement = conn.prepare(
            "WITH ranked AS (
                 SELECT id, backup_path,
                        ROW_NUMBER() OVER (
                            PARTITION BY project_id, source_path
                            ORDER BY created_at DESC, id DESC
                        ) AS rn
                   FROM backup_records
                  WHERE status = 'ok'
             )
             SELECT id, backup_path
               FROM ranked
              WHERE rn > ?1
              ORDER BY backup_path",
        )?;
        statement
            .query_map(rusqlite::params![keep.clamp(1, i64::MAX)], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };

    let mut outcome = RetentionOutcome::default();
    for (record_id, backup_path) in prunable {
        match resolve_prune_target(data_dir, &backup_path) {
            Ok(target) => match retry_transient_hold(|| fs::remove_file(&target)) {
                // The file is gone — or a previous run removed it and
                // died before the mark (the module docs' crash
                // window): converged either way, and the record is
                // marked exactly like a fresh removal (D-091's
                // `remove_copy` NotFound ruling).
                Err(source) if source.kind() == io::ErrorKind::NotFound => {
                    if mark_pruned(conn, &record_id)? {
                        outcome.pruned += 1;
                    }
                }
                Ok(()) => {
                    if mark_pruned(conn, &record_id)? {
                        outcome.pruned += 1;
                    }
                }
                // The file could not be removed: the record must not
                // claim `pruned` over a live file. Skip it for the
                // next run and keep walking (the module docs'
                // best-effort posture).
                Err(source) => outcome.failures.push(RetentionFailure {
                    record_id,
                    backup_path,
                    reason: PruneFailure::RemoveFailed(source),
                }),
            },
            // A stored cell the deletion guard refuses (the module
            // docs' SEC-006 discipline): reported, never deleted.
            Err(reason) => outcome.failures.push(RetentionFailure {
                record_id,
                backup_path,
                reason: PruneFailure::Path(reason),
            }),
        }
    }
    Ok(outcome)
}

/// Marks one record `pruned` (§6.4), only if it is still the
/// `ok` row the selection snapshot saw — the status guard makes the
/// module's "a terminal row is never rewritten" promise structural
/// rather than snapshot-dependent (the one-connection executor,
/// D-037, serializes every writer behind its mutex and cannot race
/// the two apart; BKP-005's discarded sweep has landed as a
/// producer on this table — src-tauri's `sync_phase2` inserts the
/// `E_BACKUP_FAILED` abort sweep's history rows, `discarded` per
/// deleted copy and `ok` where the deletion was refused (the
/// sync-phase-2 task, T-115) — and §10's `missing` detection is
/// still to come). Runs on the caller's connection — atomic only
/// inside the caller's transaction. Returns whether this call
/// changed the row: `false` is a converged no-op (the record
/// already left `ok`), not a failure.
fn mark_pruned(conn: &Connection, record_id: &str) -> Result<bool, StorageError> {
    let marked = conn.execute(
        "UPDATE backup_records SET status = 'pruned' WHERE id = ?1 AND status = 'ok'",
        rusqlite::params![record_id],
    )?;
    Ok(marked == 1)
}

/// Resolves a stored `backup_path` cell into the file [`apply`] may
/// remove: `data_dir` joined with the cell, once the cell passes the
/// deletion guard (the module docs) — relative, rooted at the
/// `backups` component, no `..` component, `.bak` name. Mid-path
/// `.` segments need no rule of their own: `Path::components`
/// normalizes them away, so the joined target is the same file as
/// without them and stays under the backups root.
///
/// The guard does not re-validate the full §5.5 name grammar: the
/// record was written by the app's own pipeline over builder-spelled
/// paths, and the guard's job is narrower — a corrupt or hand-edited
/// cell must not aim a deletion outside the backups tree. The strict
/// `.bak` suffix check carries that line: no path this function
/// returns can be anything but a backup-name-shaped file under
/// `backups/`.
fn resolve_prune_target(data_dir: &Path, backup_path: &str) -> Result<PathBuf, PathRejected> {
    if backup_path.is_empty() {
        return Err(PathRejected::Empty);
    }
    if !backup_path.ends_with(BACKUP_SUFFIX) {
        return Err(PathRejected::NotABackupName);
    }
    let relative = Path::new(backup_path);
    let mut components = relative.components();
    match components.next() {
        Some(Component::Normal(first)) if first == std::ffi::OsStr::new(BACKUPS_ROOT) => {}
        // A leading `/` (or a Windows prefix) makes the cell an
        // absolute path: joining would not even happen — `Path::join`
        // replaces the base — so the guard reports it before that.
        Some(Component::RootDir | Component::Prefix(_)) => {
            return Err(PathRejected::Absolute);
        }
        _ => return Err(PathRejected::NotUnderBackupsRoot),
    }
    for component in components {
        match component {
            Component::Normal(_) => {}
            // `CurDir` is unreachable here by std's normalization
            // (mid-path `.` never survives `components()`) — kept as
            // the belt under that documented behavior.
            Component::ParentDir | Component::CurDir => return Err(PathRejected::Escape),
            Component::RootDir | Component::Prefix(_) => return Err(PathRejected::Absolute),
        }
    }
    Ok(data_dir.join(relative))
}

/// What one prunable record's prune came to (the report half of
/// [`RetentionOutcome`]).
#[derive(Debug, Default)]
pub struct RetentionOutcome {
    /// Records marked `pruned` by this run — their files were removed
    /// now, or had already vanished (converged; see the module docs).
    pub pruned: u64,
    /// Records this run could NOT prune: the file stays on disk (or
    /// the stored path was refused by the deletion guard), the record
    /// keeps its `ok` status, and the next run retries. Empty when
    /// everything prunable was pruned.
    pub failures: Vec<RetentionFailure>,
}

/// One record the retention pass could not prune.
#[derive(Debug)]
pub struct RetentionFailure {
    /// The `backup_records` id.
    pub record_id: String,
    /// The stored `backup_path` cell, verbatim — the §5.5 relative
    /// layout string, for the caller's log line.
    pub backup_path: String,
    /// Why the prune did not happen.
    pub reason: PruneFailure,
}

impl fmt::Display for RetentionFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "backup record {} ({}): {}",
            self.record_id, self.backup_path, self.reason
        )
    }
}

/// Why one record's prune failed (the outcome's typed facts; the
/// caller logs them — this is not an error, see the module docs).
#[derive(Debug)]
pub enum PruneFailure {
    /// The file could not be removed — it stayed held or unreadable
    /// through the §9.5 transient-hold retries. The record keeps its
    /// `ok` status; the next run retries.
    RemoveFailed(io::Error),
    /// The stored `backup_path` cell was refused by the deletion
    /// guard: the file was never touched, and the record keeps its
    /// `ok` status until a human looks at the cell.
    Path(PathRejected),
}

impl fmt::Display for PruneFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PruneFailure::RemoveFailed(source) => {
                write!(f, "the copy could not be removed: {source}")
            }
            PruneFailure::Path(reason) => write!(f, "{reason}"),
        }
    }
}

/// Why the deletion guard refused a stored `backup_path` cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathRejected {
    /// The cell is the empty string.
    Empty,
    /// The cell names an absolute path — `Path::join` would replace
    /// the data directory with it, so the guard stops it first.
    Absolute,
    /// The cell does not start at the `backups` root of the §5.5
    /// layout, so the file is not a backup copy this service may
    /// delete.
    NotUnderBackupsRoot,
    /// A `..` component — the one escape shape `Path::components`
    /// can surface mid-path (a `.` segment is normalized away by
    /// std, and a leading `.` lands in
    /// [`PathRejected::NotUnderBackupsRoot`]). The `CurDir` half of
    /// the guard's arm is a defensive belt under std's documented
    /// normalization, not a reachable rule.
    Escape,
    /// The name is not a `.bak` backup file name.
    NotABackupName,
}

impl fmt::Display for PathRejected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathRejected::Empty => write!(f, "the stored backup path is empty"),
            PathRejected::Absolute => {
                write!(
                    f,
                    "the stored backup path is absolute, not a backups/ layout path"
                )
            }
            PathRejected::NotUnderBackupsRoot => {
                write!(
                    f,
                    "the stored backup path does not start at the backups/ root"
                )
            }
            PathRejected::Escape => {
                write!(f, "the stored backup path walks out of the backups tree")
            }
            PathRejected::NotABackupName => {
                write!(f, "the stored backup path does not name a .bak copy")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guard accepts the exact §5.5 shape the fs builders spell
    /// (the integration tests pin the same shape against the real
    /// builder output on disk).
    #[test]
    fn the_guard_accepts_the_layout_shape() {
        let target = resolve_prune_target(
            Path::new("/data"),
            "backups/0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__my-project/\
             AGENTS.md.20261119T103000123Z.ba7816bf.bak",
        )
        .expect("the layout shape must be accepted");
        assert_eq!(
            target,
            PathBuf::from(
                "/data/backups/0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__my-project/\
                 AGENTS.md.20261119T103000123Z.ba7816bf.bak"
            )
        );
    }

    /// The guard's rejections, one cell shape per rule: empty,
    /// non-`.bak`, not under `backups/`, `..` and `.` escapes, and
    /// an absolute cell (which `Path::join` would silently promote
    /// to a full replacement of the data directory).
    #[test]
    fn the_guard_rejects_cells_outside_the_layout() {
        let data_dir = Path::new("/data");
        // Mid-path `.` segments are normalized away by
        // `Path::components` itself, so they cannot smuggle anything
        // past the guard — the resolved target stays under
        // `backups/`; the leading-dot spelling lands in
        // `NotUnderBackupsRoot` below.
        let cases = [
            ("", PathRejected::Empty),
            (
                "backups/proj__slug/AGENTS.md.20261119T103000123Z.ba7816bf.txt",
                PathRejected::NotABackupName,
            ),
            (
                "notes/AGENTS.md.20261119T103000123Z.ba7816bf.bak",
                PathRejected::NotUnderBackupsRoot,
            ),
            (
                "AGENTS.md.20261119T103000123Z.ba7816bf.bak",
                PathRejected::NotUnderBackupsRoot,
            ),
            ("./backups/x.bak", PathRejected::NotUnderBackupsRoot),
            ("backups", PathRejected::NotABackupName),
            ("backups/../escaped.bak", PathRejected::Escape),
            ("/etc/passwd.bak", PathRejected::Absolute),
        ];
        for (cell, expected) in cases {
            let reason = resolve_prune_target(data_dir, cell)
                .err()
                .unwrap_or_else(|| panic!("the cell {cell:?} must be rejected"));
            assert_eq!(reason, expected, "the cell {cell:?}");
        }
    }

    /// The failure report's `Display` carries the record, the stored
    /// cell and the reason — the caller's log line verbatim.
    #[test]
    fn failure_display_carries_the_facts() {
        let failure = RetentionFailure {
            record_id: "rec-1".to_owned(),
            backup_path: "backups/p__s/x.bak".to_owned(),
            reason: PruneFailure::RemoveFailed(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "locked (injected)",
            )),
        };
        assert_eq!(
            failure.to_string(),
            "backup record rec-1 (backups/p__s/x.bak): \
             the copy could not be removed: locked (injected)"
        );
        let failure = RetentionFailure {
            record_id: "rec-2".to_owned(),
            backup_path: "../escaped.bak".to_owned(),
            reason: PruneFailure::Path(PathRejected::Escape),
        };
        assert_eq!(
            failure.to_string(),
            "backup record rec-2 (../escaped.bak): \
             the stored backup path walks out of the backups tree"
        );
    }
}
