//! The `backup_records` repository (spec §6.2 DDL, §7.6 BKP-010,
//! §9.4 phase 2; task T-115): the store's writer for one run's backup
//! copies and the dedup lookup the phase consults before making one.
//!
//! Three entry points. [`insert`] and [`last_checked`] join **the
//! caller's** transaction (D-037/D-051, the `sync_runs` shape);
//! [`all_paths`] runs on the caller's connection and joins nothing —
//! it is the read-only claim set of the orphan report (T-332/D-190):
//!
//! * [`insert`] — one row per copy a run produced. §9.4 phase 2 step 1
//!   inserts a record only after the BKP-010 size + SHA-256 check
//!   passed (`status = 'ok'`, D-122's "ok IS the checked marking"),
//!   and the abort sweep of step 2 inserts this run's already-made
//!   copies AS `discarded` — the row is the history of a copy this
//!   run created and then removed, so it lands in its terminal status
//!   directly instead of through an `ok` row a rollback could strand
//!   over a deleted file. The other two vocabulary members are other
//!   producers' (`pruned` is `backup_retention`'s UPDATE, its one
//!   owner per D-122; `missing` is §10's future detection) and no
//!   v1 caller inserts them — the typed status keeps that a fact the
//!   compiler can see rather than a comment.
//! * [`last_checked`] — the BKP-010 dedup input: the newest `ok`
//!   copy of one "project and file" pair (`(project_id,
//!   source_path)`, D-122's partition key), as `(sha256_hex,
//!   backup_path)` — the digest `eunomia-fs`'s `backup_target_file`
//!   compares the source against, and the layout path a dedup
//!   decision's own record must point at ("the record points to the
//!   existing one", BKP-010).
//! * [`all_paths`] — every `backup_path` the table claims, across all
//!   statuses: the set the orphan report subtracts from the files it
//!   lists under `backups/`, so a copy no row mentions is the only
//!   thing that report can name (D-190).
//!
//! # The writer contracts D-122 fixed (T-114's reader is the oracle)
//!
//! The retention service ships one window per `(project_id,
//! source_path)` pair over `status = 'ok'` rows ordered by
//! `created_at DESC, id DESC`, deletes the file at
//! `<data_dir>/<backup_path>` through its deletion guard, and marks
//! the record `pruned`. This module's writer is the other half of
//! that handshake and keeps the three conventions D-122 recorded:
//!
//! * `source_path` is ONE STABLE SPELLING per file — the §9.4 phase 1
//!   RESOLVED target path (post-symlink). A writer that alternates
//!   spellings splits a file's history into two retention windows
//!   (never an error, up to 2×N copies) and breaks the dedup lookup
//!   the same way, so the phase passes the resolved path's text.
//! * `backup_path` is the §5.5 RELATIVE layout string
//!   (`backups/<project_id>__<slug>/<file>.<UTC>.<hash8>.bak`,
//!   `/` separators) — `eunomia-fs`'s `BackupCopy::relative_path`
//!   spelling, built by the core builders. The UNIQUE column keeps a
//!   relocated data directory's records valid (D-122's ruling) and
//!   hands the retention guard exactly the shape it accepts.
//! * `sha256` is the full 64-character lowercase-hex digest — the
//!   same canonical spelling every BKP-010 comparison and the
//!   `hash8` name component read (D-080).
//!
//! The integration suite of this task pins the handshake end to end:
//! rows written through [`insert`] are pruned by
//! `backup_retention::apply` exactly the way T-114's own seeded
//! fixtures were — the writer and the reader agree, not just coexist.
//!
//! # Ids and times
//!
//! The id is a UUIDv7 in canonical lowercase TEXT ([`crate::values::
//! new_id`], DB-002), generated inside the joined transaction — a
//! rolled-back attempt leaves nothing behind, and the id order
//! (creation order, D-031) is the retention tiebreaker for
//! same-millisecond pairs. `created_at` is the CALLER's DB-002 UTC
//! value, not a clock read: the pipeline resolves the run's instant
//! once (D-061's posture — the copies of one run share the run's
//! stamp, and the stamp is already in the copy's name) and passes it
//! down, so the record and the file name can never disagree about
//! when the copy was made.
//!
//! # What this module does not do
//!
//! No file I/O (the copies are `eunomia-fs`'s; the abort sweep's
//! physical deletion is the pipeline's, over paths it already holds
//! from the copy step — the store never deletes what it did not
//! create), no `action_log` writes (the journal is the caller's,
//! D-051's boundary), no retention (one owner per policy, D-122), and
//! no `missing` detection (§10's future task). Rows are never
//! deleted: §6.4 keeps the history as status.

use std::fmt;

use rusqlite::Connection;

use crate::values::new_id;

/// The closed `backup_records.status` vocabulary (the §6.2 CHECK):
/// the lifecycle of one copy from its verified creation to its end.
///
/// Phase 2 writes [`BackupRecordStatus::Ok`] (a checked copy,
/// BKP-010) and [`BackupRecordStatus::Discarded`] (a copy this run
/// created and then removed in the §9.4 phase 2 step 2 sweep);
/// `backup_retention` marks [`BackupRecordStatus::Pruned`] through
/// its own UPDATE (D-122's one-owner rule), and
/// [`BackupRecordStatus::Missing`] is §10's future detection. The
/// store carries the whole vocabulary so every producer and the
/// future restore reads share one owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackupRecordStatus {
    /// The copy passed the BKP-010 check and is on disk — the only
    /// status that is a live copy (the retention window's set).
    Ok,
    /// Deleted by the retention limit (§6.4 "file deleted by limit").
    Pruned,
    /// The file vanished while the record claimed `ok` — §10's
    /// detection marks it; no v1 producer exists yet.
    Missing,
    /// Created by a run that then aborted (§9.4 phase 2 step 2) and
    /// removed with it.
    Discarded,
}

impl BackupRecordStatus {
    /// Every member, in the §6.2 CHECK's declaration order — the
    /// round-trip oracle of the vocabulary tests.
    pub const ALL: [BackupRecordStatus; 4] = [
        BackupRecordStatus::Ok,
        BackupRecordStatus::Pruned,
        BackupRecordStatus::Missing,
        BackupRecordStatus::Discarded,
    ];

    /// The stored TEXT literal (the §6.2 CHECK member).
    pub const fn as_str(&self) -> &'static str {
        match self {
            BackupRecordStatus::Ok => "ok",
            BackupRecordStatus::Pruned => "pruned",
            BackupRecordStatus::Missing => "missing",
            BackupRecordStatus::Discarded => "discarded",
        }
    }

    /// Decodes a stored (or wire) literal — the same one-owner
    /// reasoning as [`crate::sync_runs::SyncRunStatus::from_member`].
    ///
    /// # Errors
    ///
    /// Returns [`BackupRecordsError::Member`] when `member` is none
    /// of the CHECK's literals — a cell only a newer app version or
    /// tampering can hold (the D-039 corrupt-cells policy: surface
    /// it, never guess around it).
    pub fn from_member(member: &str) -> Result<Self, BackupRecordsError> {
        match member {
            "ok" => Ok(BackupRecordStatus::Ok),
            "pruned" => Ok(BackupRecordStatus::Pruned),
            "missing" => Ok(BackupRecordStatus::Missing),
            "discarded" => Ok(BackupRecordStatus::Discarded),
            _ => Err(BackupRecordsError::Member {
                column: "status",
                found: member.to_owned(),
            }),
        }
    }
}

/// Why a `backup_records` operation failed.
///
/// Hand-rolled `Display`/`Error` impls, no `thiserror` (the D-093
/// boundary rule this crate keeps); a rusqlite failure lifts into the
/// [`BackupRecordsError::Sql`] arm through a `From` impl so `?`
/// composes inside the repositories. `PartialEq` for the tests'
/// exact-value assertions (the decoded vocabulary arms compare by
/// member, not by source error).
#[derive(Debug, PartialEq)]
pub enum BackupRecordsError {
    /// A stored cell holds a value outside its closed vocabulary
    /// (`status`). Every writer goes through this repository or the
    /// schema CHECK, so this means corruption or a row written by a
    /// newer app version; the read surfaces it instead of guessing
    /// (the D-039 corrupt-cells policy).
    Member {
        /// The column the value came from.
        column: &'static str,
        /// The offending value.
        found: String,
    },
    /// The statement or query itself failed — the schema's own CHECK
    /// and foreign-key rejections (a `project_id` no `projects` row
    /// backs; a `backup_path` another record already claims through
    /// the UNIQUE column), a closed connection, a tampered schema.
    Sql(rusqlite::Error),
}

impl fmt::Display for BackupRecordsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackupRecordsError::Member { column, found } => {
                write!(f, "{found:?} is not a valid {column} value")
            }
            BackupRecordsError::Sql(err) => write!(f, "database error: {err}"),
        }
    }
}

impl std::error::Error for BackupRecordsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BackupRecordsError::Sql(err) => Some(err),
            _ => None,
        }
    }
}

/// Lets `?` lift a rusqlite failure into the typed error.
impl From<rusqlite::Error> for BackupRecordsError {
    fn from(err: rusqlite::Error) -> Self {
        BackupRecordsError::Sql(err)
    }
}

/// One new `backup_records` row — the input of [`insert`].
///
/// The column spellings are the D-122 writer contracts (the module
/// docs): `source_path` is the resolved-path text of one stable
/// spelling, `backup_path` the §5.5 relative layout string,
/// `sha256_hex` the full canonical digest. `size_bytes` is `i64`
/// because the DDL column is INTEGER and rusqlite binds no `u64` —
/// a real file's length always fits, and the conversion ruling is
/// the pipeline's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewBackupRecord<'a> {
    /// The project the copy belongs to; must exist (`projects`
    /// foreign key, `ON DELETE RESTRICT`).
    pub project_id: &'a str,
    /// The run that created the copy; `None` only for rows a future
    /// non-run producer might write (§9.4 phase 2 always has its
    /// run).
    pub sync_run_id: Option<&'a str>,
    /// The copied file's resolved path text (one stable spelling per
    /// file — the retention partition key's other half).
    pub source_path: &'a str,
    /// The copy's §5.5 relative layout path, unique in the table.
    pub backup_path: &'a str,
    /// The copy's size in bytes — the size the BKP-010 check passed.
    pub size_bytes: i64,
    /// The copy's SHA-256, 64 lowercase hex characters — the digest
    /// the BKP-010 check passed (and the name's `hash8` source).
    pub sha256_hex: &'a str,
    /// The row's status: `Ok` for a checked copy, `Discarded` for a
    /// copy the run's abort sweep removed (the module docs' producer
    /// map).
    pub status: BackupRecordStatus,
    /// The caller's DB-002 UTC timestamp — the run's one backup
    /// instant, not a per-row clock read.
    pub created_at: &'a str,
}

/// The newest checked copy of one "project and file" pair — the
/// BKP-010 dedup input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedCopy {
    /// The copy's full SHA-256, 64 lowercase hex characters — the
    /// value `backup_target_file` compares the source's digest
    /// against (exact full-digest equality, D-091).
    pub sha256_hex: String,
    /// The copy's §5.5 relative layout path — where a dedup
    /// decision's own record points ("the record points to the
    /// existing one", BKP-010).
    pub backup_path: String,
}

/// Inserts one `backup_records` row (§9.4 phase 2 step 1's record, or
/// the abort sweep's `discarded` history) and returns the generated
/// id.
///
/// The id is drawn inside the joined transaction (D-037/D-051): a
/// rolled-back attempt leaves nothing behind, and a busy-retried one
/// draws a fresh id on its fresh attempt — the `create_running`
/// contract.
///
/// # Errors
///
/// Returns [`BackupRecordsError::Sql`] when the statement fails —
/// most notably a `project_id` no `projects` row backs (the foreign
/// key refuses it), a `sync_run_id` no `sync_runs` row backs, or a
/// `backup_path` another record already claims (the UNIQUE column —
/// two copies of one run cannot share a §5.5 name, the same
/// never-overwrite rule the file side enforces with `create_new`).
pub fn insert(
    conn: &Connection,
    record: &NewBackupRecord<'_>,
) -> Result<String, BackupRecordsError> {
    let id = new_id();
    conn.execute(
        "INSERT INTO backup_records (
             id, project_id, sync_run_id, source_path, backup_path,
             size_bytes, sha256, status, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            id,
            record.project_id,
            record.sync_run_id,
            record.source_path,
            record.backup_path,
            record.size_bytes,
            record.sha256_hex,
            record.status.as_str(),
            record.created_at,
        ],
    )?;
    Ok(id)
}

/// Reads the newest `ok` copy of one "project and file" pair
/// (BKP-010's "the last backup of this file"): ordered by
/// `created_at DESC, id DESC` — the same ordering D-122 fixed for the
/// retention window, so "newest" means the same row to both halves of
/// the handshake. Terminal statuses are invisible: a `pruned`,
/// `missing` or `discarded` row is history of a gone copy, not a copy
/// (D-122's ok-only set), and the dedup must never point a record at
/// one. `None` when the pair has no checked copy yet.
///
/// # Errors
///
/// Returns [`BackupRecordsError::Sql`] when the query fails. The
/// lookup reads two TEXT cells and no vocabulary column, so
/// [`BackupRecordsError::Member`] is unreachable today — the arm
/// stays for the reads the restore screen (BKP-007) will add, the
/// same forward seam `sync_runs` documents.
pub fn last_checked(
    conn: &Connection,
    project_id: &str,
    source_path: &str,
) -> Result<Option<CheckedCopy>, BackupRecordsError> {
    let found = conn.query_row(
        "SELECT sha256, backup_path FROM backup_records
          WHERE project_id = ?1 AND source_path = ?2 AND status = 'ok'
          ORDER BY created_at DESC, id DESC
          LIMIT 1",
        rusqlite::params![project_id, source_path],
        |row| {
            Ok(CheckedCopy {
                sha256_hex: row.get(0)?,
                backup_path: row.get(1)?,
            })
        },
    );
    match found {
        Ok(copy) => Ok(Some(copy)),
        // The pair's honest "no checked copy" answer — the caller
        // decides what absence means (a first backup, D-091's
        // `None` contract).
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(err) => Err(BackupRecordsError::Sql(err)),
    }
}

/// Every `backup_path` the table claims, across **all** statuses —
/// the orphan report's known set (T-332, the owner ruling of
/// 2026-11-24).
///
/// All statuses on purpose: a `pruned` row's file is gone, so
/// including it can only keep a path out of the orphan set that no
/// file carries any more, while a `discarded` row is the abort
/// sweep's history of a copy this run itself removed. Excluding
/// either would widen the report, never narrow it wrongly — but
/// including them costs nothing and states the contract the report
/// relies on: a file is orphaned only when **no** row mentions it.
/// The order is the stored cell order (`backup_path`, then the
/// engine's tiebreak) so the read is deterministic under test.
///
/// # Errors
///
/// Returns [`BackupRecordsError::Sql`] when the query fails; the
/// caller's report degrades with a log line (it is housekeeping, not
/// a command).
pub fn all_paths(conn: &Connection) -> Result<Vec<String>, BackupRecordsError> {
    let mut statement = conn.prepare(
        "SELECT backup_path FROM backup_records
          ORDER BY backup_path, id",
    )?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(BackupRecordsError::Sql)
}

#[cfg(test)]
mod tests {
    //! The pure halves — the vocabulary's literal round trip and the
    //! error Display shapes. The database behavior (insert shapes,
    //! constraint refusals, the dedup lookup, the retention
    //! handshake) lives in `tests/backup_records.rs` on a real
    //! migrated database, the house split for repository modules.

    use super::*;

    /// The stored literals are the §6.2 CHECK members, verbatim, in
    /// both directions — the literals are the oracle, not the
    /// variant names (the `sync_runs` vocabulary test's reasoning).
    #[test]
    fn the_vocabulary_round_trips_the_schema_literals() {
        assert_eq!(
            BackupRecordStatus::ALL
                .map(|status| status.as_str())
                .map(str::to_owned),
            ["ok", "pruned", "missing", "discarded"]
        );
        for status in BackupRecordStatus::ALL {
            assert_eq!(
                BackupRecordStatus::from_member(status.as_str()),
                Ok(status),
                "the literal of {status:?} must decode back to it"
            );
        }
    }

    /// A literal outside the CHECK is the typed corrupt-cell refusal,
    /// never a guess — with the column named and the value carried.
    #[test]
    fn a_foreign_literal_is_refused_with_the_column_named() {
        assert_eq!(
            BackupRecordStatus::from_member("OK"),
            Err(BackupRecordsError::Member {
                column: "status",
                found: "OK".to_owned(),
            })
        );
    }

    /// The error Display shapes callers log at the edges.
    #[test]
    fn error_display_shapes() {
        assert_eq!(
            BackupRecordsError::Member {
                column: "status",
                found: "pending".to_owned(),
            }
            .to_string(),
            "\"pending\" is not a valid status value"
        );
        let sql = BackupRecordsError::Sql(rusqlite::Error::InvalidParameterCount(2, 1));
        assert_eq!(
            sql.to_string(),
            "database error: Wrong number of parameters passed to query. Got 2, needed 1"
        );
    }
}
