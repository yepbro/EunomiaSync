//! The Projects module's `project_update` command (spec §7.10, T-106;
//! PRJ-008/009/011/021, §6.9, SYS-005, LOG-001…004; the contract's
//! seven resolved readings live in DECISIONS.md D-201).
//!
//! One id-addressed PATCH over the four editable fields — name,
//! label, color, description — carried by ONE `Db::run` transaction
//! (D-037) together with the single `project.changed` journal entry:
//! commit or roll back together (D-051, §6.9's "an entity is never
//! saved partly"). Every absent field stays unchanged (the store's
//! own [`ProjectUpdate`] semantics); the columns the command does not
//! own — `path`, `path_key`, `added_via`, `state`, `sort_order`,
//! `last_synced_*` — are untouched by construction: PRJ-008's "the
//! path cannot be changed" is answered by the contract itself (there
//! is no path argument to refuse), and the store's update type has no
//! members for them. The scheme/file-name/extra-file half of the
//! settings modal is T-107's own arm (DB-023's watch-set inputs).
//!
//! No file I/O at all (D-201): none of the four fields feeds the
//! watch set (§5.7 SYS-007's own list ties watch changes to additions,
//! scheme or file-name changes and archivals), so T-279's
//! `refresh_after_sync` wiring is deliberately absent here — the
//! retrofit card owns it, and a pass over an unchanged set would be
//! file I/O with no observable change (D-194's own decline for the
//! archive). The SYS-005 gate runs as the first act, before anything
//! else: while a sync holds the global `sync_running` flag, the
//! command answers the typed `E_SYNC_RUNNING` refusal and writes
//! nothing.

use eunomia_core::project_validators::{
    validate_color, validate_description, validate_label, validate_name,
};
use eunomia_store::action_log::{
    Action, Entity, EntityType, Level, LogEntry, Outcome, Visibility, insert,
};
use eunomia_store::projects::{ProjectState, ProjectUpdate};
use tauri::{AppHandle, Runtime};

use crate::commands::{
    activity_log_to_app_error, bounded_echo, managed_db, projects_store_to_app_error,
    reject_during_sync, rejected_field,
};
use crate::errors::{AppError, ErrorCode};
use crate::logging;

/// Updates the editable fields of the project with this id (PRJ-008:
/// name, label, color, description) with §6.9 validation, in one
/// transaction that also journals the single `project.changed` entry
/// (LOG-001…004, D-201 point 5).
///
/// Id-addressed like `project_archive` (the id comes from
/// `projects_list`'s rows), and a PATCH: each field is
/// `Option<String>` — absent (`null`) leaves the field unchanged, the
/// store's own `ProjectUpdate` semantics — so a stale screen that
/// re-sends one field cannot wipe the other three, and the modal's
/// full save is the same contract. The answer is `()`: the screen
/// re-reads through the already-landed `projects_list`.
///
/// A duplicate name is SAVED, not refused (§6.9's "Duplicates are
/// allowed with a warning"): the warning is the modal's live field
/// validation (T-160), computable from the names `projects_list`
/// already serves — the command carries no warning arm.
///
/// Validation order (§6.9, all-or-nothing): the present fields are
/// validated through the shared core validators in table order
/// (name, label, description, color) BEFORE the database is resolved
/// — the first invalid field answers with the `E_VALIDATION`
/// envelope naming it, and nothing is saved (the card's DoD). The
/// name, label and color refusals echo the caller's field bounded
/// (D-115's cap; the `rejected_title_field` shape); the description
/// refusal carries NO value — stored prose is the one thing SEC-008
/// keeps out of envelopes (the variant-text rule), and the
/// validator's reason already carries the measured length.
///
/// An id no ACTIVE row matches — unknown, or archived since the
/// modal opened (PRJ-008's pencil rides an active column; the D-115
/// reorder precedent: "unknown OR archived: a stale screen's list") —
/// is the typed `E_VALIDATION` refusal naming `project_id` with the
/// id echoed bounded; nothing is written.
///
/// The verified no-op (D-201 point 5): a send whose present fields
/// all equal the stored values is stripped to an empty patch BEFORE
/// the store call, so nothing is written, `updated_at` is not
/// restamped (the store's own empty-update rule), and nothing is
/// journaled.
///
/// While a sync holds the global `sync_running` flag, the command
/// answers the typed `E_SYNC_RUNNING` envelope instead (SYS-005, the
/// card's own line; [`reject_during_sync`] — a check, never an
/// acquire), and the screen's retry after the run ends converges.
/// Recovery mode answers the honest `NoDatabase` fallback (the flag
/// is down there by construction, so the answer comes from
/// [`managed_db`]).
///
/// Failures reach the log with their §10 code
/// ([`logging::project_update_failed`]); a successful save is the
/// journal entry itself (the D-051 mirror policy — info+user+ok is
/// not mirrored to the file log).
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn project_update<R: Runtime>(
    app: AppHandle<R>,
    project_id: String,
    name: Option<String>,
    label: Option<String>,
    color: Option<String>,
    description: Option<String>,
) -> Result<(), AppError> {
    project_update_impl(app, project_id, name, label, color, description)
        .await
        .inspect_err(logging::project_update_failed)
}

/// The body of [`project_update`], split so the one logging edge
/// covers every failure path (the `activity_list` shape) — including
/// the SYS-005 refusal, which fires before anything else.
#[allow(clippy::too_many_arguments)]
async fn project_update_impl<R: Runtime>(
    app: AppHandle<R>,
    project_id: String,
    name: Option<String>,
    label: Option<String>,
    color: Option<String>,
    description: Option<String>,
) -> Result<(), AppError> {
    // SYS-005 (§5.4): the mode gate runs before everything — while a
    // sync holds `sync_running`, the update is refused with
    // `E_SYNC_RUNNING` whatever the input is (see `reject_during_sync`).
    reject_during_sync(&app)?;
    // §6.9 first (the `instruction_update` ordering): every PRESENT
    // field validates before the database is resolved, in the spec
    // table's own order — name, label, description, color. A refused
    // field means the whole operation never runs: an entity is never
    // saved partly (§6.9's closing rule, the card's DoD).
    let name =
        match name {
            Some(sent) => Some(validate_name(&sent).map_err(|source| {
                rejected_field("name", &bounded_echo(&sent), &source.to_string())
            })?),
            None => None,
        };
    let label = match label {
        Some(sent) => Some(validate_label(&sent).map(str::to_owned).map_err(|source| {
            rejected_field("label", &bounded_echo(&sent), &source.to_string())
        })?),
        None => None,
    };
    // The description's own validator IS the §6.9 rule (the silent
    // NUL strip, the post-strip measure); its refusal echoes no
    // value — see the command docs (SEC-008).
    let description = match description {
        Some(sent) => Some(validate_description(&sent).map_err(|source| {
            let mut error = AppError::new(ErrorCode::Validation);
            error
                .params
                .insert("field".to_owned(), "description".to_owned());
            error.params.insert("reason".to_owned(), source.to_string());
            error
        })?),
        None => None,
    };
    // `validate_color` is the only gate a color passes (DB-040; the
    // §6.2 column has no CHECK). Its error's own Display interpolates
    // the sent value — caller input that must not ride the envelope
    // whole — so this refusal states the class and echoes the value
    // bounded instead (Rule 8; the `activity_list` action-filter
    // shape).
    let color = match color {
        Some(sent) => Some(validate_color(&sent).map_err(|_| {
            rejected_field(
                "color",
                &bounded_echo(&sent),
                "the color is not one of the 12 palette colors (DB-040)",
            )
        })?),
        None => None,
    };
    let database = managed_db(&app)?;
    database
        .run(move |transaction| {
            // The row first: the by-id read decides the stale-screen
            // branch and carries the stored values the patch is
            // compared against. The refusal echoes the caller's id
            // bounded (D-115's cap: a webview string is unbounded by
            // definition, and it rides the envelope and the §10 log
            // line otherwise).
            let project = eunomia_store::projects::get(transaction, &project_id)
                .map_err(projects_store_to_app_error)?
                .filter(|project| project.state == ProjectState::Active)
                .ok_or_else(|| {
                    rejected_field(
                        "project_id",
                        &bounded_echo(&project_id),
                        "no active project has this id",
                    )
                })?;
            // The verified no-op (D-201 point 5): a present field equal
            // to its stored value contributes nothing — the patch is
            // stripped to the fields that actually change, and an empty
            // patch writes, restamps and journals nothing (the
            // D-188/D-115 precedent; the store would restamp
            // `updated_at` for any present field).
            let update = ProjectUpdate {
                name: name
                    .as_deref()
                    .filter(|value| *value != project.name.as_str()),
                label: label
                    .as_deref()
                    .filter(|value| *value != project.label.as_str()),
                color: color
                    .as_ref()
                    .map(|palette| palette.as_str())
                    .filter(|value| *value != project.color.as_str()),
                description: description
                    .as_deref()
                    .filter(|value| *value != project.description.as_str()),
                ..ProjectUpdate::default()
            };
            if update == ProjectUpdate::default() {
                return Ok(());
            }
            let updated = eunomia_store::projects::update(transaction, &project_id, &update)
                .map_err(projects_store_to_app_error)?;
            // LOG-001…004 ("changing … projects") inside the
            // operation's own transaction (D-051, D-119's recorded
            // member for the update family): ONE attributable entry —
            // entity and project carry the row's id; no details (the
            // `register_project`/`project_archived` shape: the row
            // itself holds every field this command writes).
            let mut entry = LogEntry::new(
                Action::ProjectChanged,
                Level::Info,
                Visibility::User,
                Outcome::Ok,
            );
            entry.entity = Some(Entity {
                kind: EntityType::Project,
                id: updated.id.as_str(),
            });
            entry.project_id = Some(updated.id.as_str());
            insert(transaction, &entry).map_err(activity_log_to_app_error)?;
            // The D-051 mirror contract: one call per written entry
            // (info+user+ok is the unmirrored shape today).
            logging::action_logged(&entry);
            Ok(())
        })
        .await
}
