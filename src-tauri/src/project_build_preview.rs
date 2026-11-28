//! The Projects module's `project_build_preview` command (spec §7.10,
//! MAIN-033; task T-358): the read that answers what the next sync
//! would write — the project's target path and the current managed
//! content, §9.2/§9.3 built from a §9.1 snapshot, WITHOUT the
//! creation marker (scheme B answers the managed block only).
//!
//! The File-preview modal (MAIN-033, the UI task's half) renders this
//! text through `markdown_preview` beside the target path in
//! monospace; the command itself is pure data — a read, the
//! `sync_status`/`library_get` posture: **no SYS-005 gate** (§5.4's
//! list is the project-*changing* commands; a preview during a live
//! sync is the panel's own moment-of-read fact — the snapshot the
//! next run builds may differ, exactly as a mid-run `projects_list`
//! may), **no journal** (reading is not an event, D-051), and
//! recovery mode answers the honest `NoDatabase` fallback through
//! [`managed_db`] like every database read.
//!
//! # The composition — the build_snapshot landing zone
//!
//! One `Db::run` read transaction ([`eunomia_store::build_snapshot::read`],
//! T-100/D-105 — SYS-005's one-transaction snapshot, the store's
//! single mapping into core's value), then the build over the
//! immutable snapshot only:
//!
//! - the project by id among the snapshot's **active** projects (the
//!   store's scope already filtered the archived rows out, DB-014);
//!   an id no active project answers — unknown, or archived since the
//!   column rendered — is the typed `E_VALIDATION` refusal naming
//!   `project_id`, the `project_update` shape, decided INSIDE the
//!   transaction exactly where the by-id read runs;
//! - the §9.2 body ([`eunomia_core::body::build_body`]) — headings by
//!   ALG-020, no instruction names (ALG-022), each cell's effective
//!   text with the ALG-023 composition for extend cells (the T-356
//!   composition flows through `build_body` unchanged);
//! - the §9.3 assembly ([`eunomia_core::target_file::managed_text`],
//!   D-064's markerless seam): scheme A splices the D-21 reading
//!   lines around the body, B and C take the body alone — which IS
//!   the whole of "scheme B answers the managed block only": the
//!   preview never splices the file's outside, it answers the block
//!   the next run would write between the markers;
//! - the target path — §9.4 phase 1 step 2's spelling
//!   (`<root>/<base_name>` or `<root>/<STEM>.project.md`), the same
//!   join [`eunomia_core::content_build`] spells for the sync, as
//!   TEXT through phase 0's single spelling
//!   ([`crate::sync_phase0::target_path_text`]).
//!
//! The answer is byte-identical with the sync's own build by
//! construction: every seam above is the one the pipeline calls
//! (`build_projects` composes the same
//! `build_body` + `extra_file_lines` + `managed_text` + join per
//! project), and the marker is the one deliberate difference — a
//! per-run wall-clock line (ALG-032) that does not exist yet and that
//! MAIN-033's sentence excludes. No build path is forked here; the
//! test battery pins the identity against a REAL run (phases 0–3 over
//! a temp repository, the written file compared byte for byte with
//! the preview's text).
//!
//! # Resource posture
//!
//! The build runs inside the `Db::run` closure, which the executor
//! already runs on the blocking pool (SYS-001, `db.rs`). The work is
//! one project's share of the whole-library pass phase 0 owns: linear
//! in the snapshot's item and assignment counts plus the assigned
//! texts' lengths (rule 6's stated complexity — `build_body` walks the
//! items once with one pass over the assignments, `managed_text`
//! joins strings), so holding the executor for it is the same order
//! every command's transaction already holds — phase 0's
//! spawn-blocking-after-the-transaction shape (D-096) exists for the
//! ALL-projects pass, not for one project's linear share, and the
//! landing-zone note in `build_snapshot.rs` names this very
//! composition as the read-side consumer.

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Runtime};

use eunomia_core::backup_name::target_file_name;
use eunomia_core::body::build_body;
use eunomia_core::extra_files::extra_file_lines;
use eunomia_core::target_file::managed_text;
use eunomia_store::build_snapshot::read as read_build_snapshot;

use crate::commands::{bounded_echo, managed_db, rejected_field};
use crate::dirty_recalc::build_snapshot_to_app_error;
use crate::errors::AppError;
use crate::logging;
use crate::sync_phase0::target_path_text;

/// The `project_build_preview` answer (§7.10, MAIN-033): where the
/// next sync would write, and the managed content it would lay down
/// there minus the creation marker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct BuildPreview {
    /// The target file's path — §9.4 phase 1 step 2's spelling: the
    /// stored (canonical) project root joined with the scheme's file
    /// name, BEFORE any symlink resolution (the preview is a read; it
    /// never touches the filesystem). UTF-8 TEXT by construction —
    /// the stored path was validated at its write boundary and the
    /// file name is one of four static ASCII literals.
    pub(crate) target_path: String,
    /// The managed content (§9.2/§9.3), creation marker excluded:
    /// scheme A the D-21 reading lines spliced around the §9.2 body,
    /// schemes B and C the body alone (scheme B's managed block —
    /// the block the next run writes between the markers, never the
    /// file's outside). An empty build is the empty string, the text
    /// MAIN-033's "a note instead of an empty document" note replaces
    /// on the UI side.
    pub(crate) text: String,
}

/// Answers what the next sync would write to the project's managed
/// area (spec §7.10 `project_build_preview`, MAIN-033): the target
/// path and the current managed content — §9.2/§9.3 built from one
/// §9.1 snapshot read, WITHOUT the creation marker; scheme B answers
/// the managed block only.
///
/// A read: no SYS-005 gate (the `sync_status` posture — SYS-005's
/// list is the project-*changing* commands), no journal, no write of
/// any kind. The one refusal besides the executor's own is the
/// `project_update` id shape: an id no ACTIVE project answers —
/// unknown, or archived — is the typed `E_VALIDATION` envelope naming
/// `project_id` with the caller's id echoed bounded (D-115's cap),
/// and nothing is built. Recovery mode answers the honest
/// `NoDatabase` fallback.
///
/// Failures reach the log with their §10 code
/// ([`logging::project_build_preview_failed`]); a successful read is
/// never journaled and logs nothing (reading is not an event — the
/// `activity_list` rationale).
#[tauri::command]
#[specta::specta]
pub(crate) async fn project_build_preview<R: Runtime>(
    app: AppHandle<R>,
    project_id: String,
) -> Result<BuildPreview, AppError> {
    project_build_preview_impl(app, project_id)
        .await
        .inspect_err(logging::project_build_preview_failed)
}

/// The body of [`project_build_preview`], split the `projects_list`
/// way so the one logging edge covers every failure path — including
/// the id refusal, which fires inside the transaction.
async fn project_build_preview_impl<R: Runtime>(
    app: AppHandle<R>,
    project_id: String,
) -> Result<BuildPreview, AppError> {
    managed_db(&app)?
        .run(move |tx| {
            // The one-transaction snapshot (SYS-005/D-105): the store's
            // single mapping into core's value, the read the sync itself
            // builds from. Its project list is the ACTIVE list (DB-014),
            // so one lookup answers the id both ways MAIN-033's sentence
            // names — unknown, or archived since the column rendered. The
            // refusal is the `project_update` envelope: `project_id`,
            // the caller's id echoed bounded, the class-only reason.
            let snapshot = read_build_snapshot(tx).map_err(build_snapshot_to_app_error)?;
            let project = snapshot
                .projects()
                .iter()
                .find(|known| known.id.as_str() == project_id)
                .ok_or_else(|| {
                    rejected_field(
                        "project_id",
                        &bounded_echo(&project_id),
                        "no active project has this id",
                    )
                })?;
            // The §9.2 body over the same snapshot. `None` means "not
            // among the snapshot's projects" — the same list the lookup
            // just answered — so it is unreachable by construction; the
            // expect is D-061's reasoned-panic posture, never a silent
            // default.
            let body = build_body(&snapshot, &project.id)
                .expect("the project was found in the snapshot's own project list");
            // The §9.3 assembly, D-064's markerless seam: the same
            // `extra_file_lines` + `managed_text` pair `build_projects`
            // spells per project (scheme A's reading lines around the
            // body — the argument order is project-then-local, §7.5's tie
            // order), minus everything the marker owns. The marker is
            // ALG-032's per-run wall-clock line and deliberately does not
            // exist here.
            let settings = &project.settings;
            let lines = extra_file_lines(
                settings.base_name,
                settings.extra_project,
                settings.extra_local,
            );
            let text = managed_text(settings.scheme, &body, &lines);
            // The target path — the same §9.4 phase-1-step-2 join the
            // sync's build spells (`content_build`'s per-project shape),
            // through phase 0's single TEXT spelling of it.
            let target_path = project
                .root
                .join(target_file_name(settings.scheme, settings.base_name));
            Ok(BuildPreview {
                target_path: target_path_text(&target_path)?,
                text,
            })
        })
        .await
}
