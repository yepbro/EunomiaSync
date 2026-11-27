//! The Projects module's `project_update` command (spec §7.10, T-106;
//! PRJ-008/009/011/021, §6.9, SYS-005, LOG-001…004; the contract's
//! seven resolved readings live in DECISIONS.md D-201, the
//! write-configuration half's nine in D-204 — T-107, PRJ-012, DB-023,
//! §7.5, D-01, D-03, SYS-007).
//!
//! One id-addressed PATCH over the settings modal's whole field set —
//! name, label, color, description (§6.9), plus the PRJ-012 half:
//! scheme, file name, and the two extra-file options (§7.5) — carried
//! by ONE `Db::run` transaction (D-037) together with the single
//! `project.changed` journal entry: commit or roll back together
//! (D-051, §6.9's "an entity is never saved partly"). Every absent
//! field stays unchanged (the store's own [`ProjectUpdate`]
//! semantics); the columns the command does not own — `path`,
//! `path_key`, `added_via`, `state`, `sort_order`, `last_synced_*` —
//! are untouched by construction: PRJ-008's "the path cannot be
//! changed" is answered by the contract itself (there is no path
//! argument to refuse), and the store's update type has no members
//! for them.
//!
//! No file writes at all (D-194's posture, D-204 point 6): DB-023's
//! "the old file with the previous name is not deleted" is a
//! prohibition the command satisfies by never writing the disk — the
//! next sync writes the new target file, and the old file's fate is
//! the user's own (§7.5 assigns no cleanup anywhere). What the row
//! change DOES carry is the DB-023 dirty clause, decided where it is
//! counted: the stored `last_synced_hash` was computed over the old
//! build configuration, DB-020's formula hashes the scheme, the
//! target path and the extra-file options, and the DB-022 service's
//! next pass recalculates and answers dirty (D-204 point 5 — nothing
//! is written for it here).
//!
//! The one deliberate exception to the command's otherwise complete
//! lack of file-system interaction is the watch refresh
//! (D-204 point 7, SYS-007/D-144 — T-279's arm, the OPPOSITE reading
//! from D-201 point 7): the four PRJ-012 fields ARE the watch pass's
//! verdict inputs (`ProjectFact` consumes exactly them), so a
//! committed patch that changed at least one of them awaits ONE full
//! watch pass through the bounded mutation seam
//! ([`file_watch::refresh_after_mutation`], D-156 — the same refresh
//! path `project_add_manual`'s committed arm runs) AFTER the
//! transaction, never-fatal: a degraded pass logs its class and the
//! save stands. A patch that changed no watch input — the §6.9 fields
//! only, or a verified no-op — runs no refresh (a pass over unchanged
//! inputs would read files with no observable change, D-201 point
//! 7's own reading). The SYS-005 gate runs as the first act, before
//! anything else: while a sync holds the global `sync_running` flag,
//! the command answers the typed `E_SYNC_RUNNING` refusal and writes
//! nothing.

use eunomia_core::project_validators::{
    validate_color, validate_description, validate_label, validate_name,
};
use eunomia_store::action_log::{
    Action, Entity, EntityType, Level, LogEntry, Outcome, Visibility, insert,
};
use eunomia_store::projects::{ProjectState, ProjectUpdate};
use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};
use serde::Deserialize;
use specta::Type;
use tauri::{AppHandle, Runtime};

use crate::commands::{
    activity_log_to_app_error, bounded_echo, managed_db, projects_store_to_app_error,
    reject_during_sync, rejected_field,
};
use crate::errors::{AppError, ErrorCode};
use crate::file_watch;
use crate::logging;

/// One extra-file option of the `project_update` patch (§7.5 scheme A,
/// D-01): the include flag and where the "read this file" line goes,
/// in the §6.5 JSON object shape `{"enabled": …, "position": …}` the
/// settings table and the core `ExtraFile` serde form share.
///
/// `position` arrives as the raw wire string, not a typed enum: it is
/// a closed-vocabulary literal the command itself parses, so an
/// off-list member answers the TYPED `E_VALIDATION` envelope naming
/// the field (the color's precedent — a specta-typed enum arg would
/// move the refusal to Tauri's arg-deserialization boundary, an
/// untyped string error with no `message_key` for UI-011 to
/// localize). `enabled` is a plain `bool`: a non-boolean shape is an
/// invoke-level deserialization failure, exactly as a non-string
/// `name` already is (the D-003 typed-IPC boundary's own behavior).
/// Unknown members are refused the same way
/// (`deny_unknown_fields`, the core `ExtraFile` serde shape:
/// missing members, unknown members and wrong types are rejected,
/// not defaulted).
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExtraFilePatch {
    /// Whether the line is added at all.
    pub(crate) enabled: bool,
    /// Where the line is added — `start` or `end`, parsed by
    /// [`parse_extra_file`].
    pub(crate) position: String,
}

/// Updates the editable fields of the project with this id (PRJ-008:
/// name, label, color, description; PRJ-012: the scheme, the file
/// name and the two extra-file options) with §6.9 validation, in one
/// transaction that also journals the single `project.changed` entry
/// (LOG-001…004, D-201 point 5, D-204 point 8).
///
/// Id-addressed like `project_archive` (the id comes from
/// `projects_list`'s rows), and a PATCH: each field is optional —
/// absent (`null`) leaves the field unchanged, the store's own
/// [`ProjectUpdate`] semantics — so a stale screen that re-sends one
/// field cannot wipe the others, and the modal's full save is the
/// same contract. The answer is `()`: the screen re-reads through the
/// already-landed `projects_list`.
///
/// A duplicate name is SAVED, not refused (§6.9's "Duplicates are
/// allowed with a warning"): the warning is the modal's live field
/// validation (T-160), computable from the names `projects_list`
/// already serves — the command carries no warning arm.
///
/// Validation order (§6.9, all-or-nothing): the present fields are
/// validated before the database is resolved — the §6.9 fields in the
/// spec table's own order (name, label, description, color), then the
/// PRJ-012 half in PRJ-012's own sentence order (scheme, file name,
/// `extra_local`, `extra_project` — §6.9's table carries only the
/// file name among them; D-204 point 2) — and the first invalid field
/// answers with the `E_VALIDATION` envelope naming it, so nothing is
/// saved (the card's DoD). The name, label and color refusals echo
/// the caller's field bounded (D-115's cap; the
/// `rejected_title_field` shape); the description refusal carries NO
/// value — stored prose is the one thing SEC-008 keeps out of
/// envelopes; the scheme, file-name and extra-position refusals echo
/// the caller's literal bounded (closed-vocabulary words, the
/// color/action-filter precedent).
///
/// The extra-file fields apply to the ROW, not the scheme window
/// (D-204 point 3, the spec's own words): the §6.2 columns are NOT
/// NULL on every row, §6.5 seeds `default.extra_*` for every new
/// project whatever the scheme, and the BUILDER — not the store — is
/// what ignores them under schemes B and C (ALG-030's "(A only)";
/// PRJ-008's "extra files only in scheme A" names the modal's
/// scheme-conditional block). A B/C row therefore saves an extra-file
/// patch like any other: stored, journaled, and dirtied through
/// DB-020's no-qualifier formula.
///
/// An id no ACTIVE row matches — unknown, or archived since the
/// modal opened (PRJ-008's pencil rides an active column; the D-115
/// reorder precedent: "unknown OR archived: a stale screen's list") —
/// is the typed `E_VALIDATION` refusal naming `project_id` with the
/// id echoed bounded; nothing is written.
///
/// The verified no-op (D-201 point 5, D-204 point 9): a send whose
/// present fields all equal the stored values — the extras compared
/// as whole enabled-plus-position pairs — is stripped to an empty
/// patch BEFORE the store call, so nothing is written, `updated_at`
/// is not restamped (the store's own empty-update rule), nothing is
/// journaled, and no watch refresh runs.
///
/// While a sync holds the global `sync_running` flag, the command
/// answers the typed `E_SYNC_RUNNING` envelope instead (SYS-005, the
/// card's own line; [`reject_during_sync`] — a check, never an
/// acquire), and the screen's retry after the run ends converges.
/// Recovery mode answers the honest `NoDatabase` fallback (the flag
/// is down there by construction, so the answer comes from
/// [`managed_db`]; the watch refresh is unreachable without a
/// database by the same ordering).
///
/// After the transaction commits, IF the patch changed at least one
/// watch-verdict input (scheme, file name, an extra-file option —
/// SYS-007's own list, the fields the watch pass's `ProjectFact`
/// consumes), the command awaits ONE full watch pass through the
/// bounded mutation seam (D-144/D-156, D-204 point 7):
/// never-fatal aftermath — a refused or expired pass logs its class
/// through [`logging::file_watch_mutation_refresh_degraded`] and the
/// save stands, and the next watch anchor re-derives the set
/// (D-144's convergence). A patch that changed none of them (the
/// §6.9 fields only, or the verified no-op) runs no refresh.
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
    scheme: Option<String>,
    base_name: Option<String>,
    extra_local: Option<ExtraFilePatch>,
    extra_project: Option<ExtraFilePatch>,
) -> Result<(), AppError> {
    project_update_impl(
        app,
        project_id,
        name,
        label,
        color,
        description,
        scheme,
        base_name,
        extra_local,
        extra_project,
    )
    .await
    .inspect_err(logging::project_update_failed)
}

/// The body of [`project_update`], split so the one logging edge
/// covers every failure path (the `activity_list` shape) — including
/// the SYS-005 refusal, which fires before anything else. The watch
/// refresh's never-fatal degrade is NOT a failure path: it logs its
/// own class and the answer stays `Ok`.
#[allow(clippy::too_many_arguments)]
async fn project_update_impl<R: Runtime>(
    app: AppHandle<R>,
    project_id: String,
    name: Option<String>,
    label: Option<String>,
    color: Option<String>,
    description: Option<String>,
    scheme: Option<String>,
    base_name: Option<String>,
    extra_local: Option<ExtraFilePatch>,
    extra_project: Option<ExtraFilePatch>,
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
    // The PRJ-012 half (D-204 point 2), in PRJ-012's own sentence
    // order after the §6.9 half: the scheme (§7.5's closed A/B/C),
    // the file name (§6.9's "a value from the list" over D-03's
    // two members), then the two extra-file options (§6.5's
    // enabled-plus-position pairs). Each closed vocabulary is
    // case-sensitive like the schema CHECKs, refuses the whole save
    // naming its field, and echoes the caller's literal bounded
    // (D-115's cap — the wire string is unbounded by definition).
    let scheme = match scheme {
        Some(sent) => Some(parse_scheme(&sent)?),
        None => None,
    };
    let base_name = match base_name {
        Some(sent) => Some(parse_base_name(&sent)?),
        None => None,
    };
    let extra_local = match extra_local {
        Some(ref sent) => Some(parse_extra_file("extra_local", sent)?),
        None => None,
    };
    let extra_project = match extra_project {
        Some(ref sent) => Some(parse_extra_file("extra_project", sent)?),
        None => None,
    };
    let database = managed_db(&app)?;
    let watch_inputs_changed = database
        .run_with_mirror(move |transaction, mirror| {
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
            // The verified no-op (D-201 point 5, D-204 point 9): a
            // present field equal to its stored value contributes
            // nothing — the patch is stripped to the fields that
            // actually change (the extras as whole enabled-plus-
            // position pairs: a disabled-but-different position is a
            // real row change, hashed by DB-020 and re-derived by the
            // watch pass), and an empty patch writes, restamps,
            // journals and refreshes nothing (the D-188/D-115
            // precedent; the store would restamp `updated_at` for any
            // present field).
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
                scheme: scheme.filter(|value| *value != project.scheme),
                base_name: base_name.filter(|value| *value != project.base_name),
                extra_local: extra_local.filter(|value| *value != project.extra_local),
                extra_project: extra_project.filter(|value| *value != project.extra_project),
            };
            if update == ProjectUpdate::default() {
                return Ok(false);
            }
            // D-204 point 7's boundary, decided on the STRIPPED patch:
            // the watch-verdict inputs are exactly the four PRJ-012
            // fields (SYS-007's own list; the watch pass's
            // `ProjectFact` consumes them), so the refresh belongs to
            // the committed patch that moved at least one of them —
            // never to a §6.9-only change or a no-op.
            let watch_inputs_changed = update.scheme.is_some()
                || update.base_name.is_some()
                || update.extra_local.is_some()
                || update.extra_project.is_some();
            let updated = eunomia_store::projects::update(transaction, &project_id, &update)
                .map_err(projects_store_to_app_error)?;
            // LOG-001…004 ("changing … projects") inside the
            // operation's own transaction (D-051, D-119's recorded
            // member for the update family): ONE attributable entry —
            // entity and project carry the row's id; no details (the
            // `register_project`/`project_archived` shape: the row
            // itself holds every field this command writes, the
            // write-configuration half included — D-204 point 8).
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
            // The T-324/D-228 mirror contract: record inside the
            // transaction, flush only after the commit (info+user+ok
            // is the unmirrored shape today).
            mirror.record(&entry);
            Ok(watch_inputs_changed)
        })
        .await?;
    // SYS-007/D-144, T-279's arm (D-204 point 7): the committed
    // watch-input change awaits ONE full watch pass through the
    // bounded mutation seam — the same awaited pass (command, ack,
    // service pass) `project_add_manual`'s committed arm and the sync
    // pipeline's aftermath run, with the interactive wait budget
    // D-156/T-280 added. NEVER-FATAL aftermath: the save stands
    // committed whatever the pass answers — a refused send (a watcher
    // backend that never started), a failed pass or an expired
    // budget degrades with the class-only warn, and the next watch
    // anchor re-derives the set without loss (D-144's convergence:
    // every full pass re-derives the watch set from the project
    // table). Strictly AFTER the transaction — the journal insert
    // stays inside it, the refresh never is.
    if watch_inputs_changed && let Err(refused) = file_watch::refresh_after_mutation(&app).await {
        logging::file_watch_mutation_refresh_degraded(&refused);
    }
    Ok(())
}

/// Parses the scheme literal (§7.5's closed `A`/`B`/`C`,
/// case-sensitive like the `projects.scheme` CHECK) into the store
/// member, refusing any other literal with the typed envelope
/// (D-204 point 2).
fn parse_scheme(literal: &str) -> Result<Scheme, AppError> {
    match literal {
        "A" => Ok(Scheme::A),
        "B" => Ok(Scheme::B),
        "C" => Ok(Scheme::C),
        _ => Err(rejected_field(
            "scheme",
            &bounded_echo(literal),
            "the scheme must be one of A, B, C (spec 7.5)",
        )),
    }
}

/// Parses the file-name literal (§6.9's "a value from the list" over
/// D-03's two members, case-sensitive like the `projects.base_name`
/// CHECK) into the store member, refusing any other literal the same
/// way — "path and extension are not typed in" is why the vocabulary
/// is closed at all.
fn parse_base_name(literal: &str) -> Result<BaseName, AppError> {
    match literal {
        "AGENTS.md" => Ok(BaseName::AgentsMd),
        "CLAUDE.md" => Ok(BaseName::ClaudeMd),
        _ => Err(rejected_field(
            "base_name",
            &bounded_echo(literal),
            "the file name must be AGENTS.md or CLAUDE.md (spec D-03)",
        )),
    }
}

/// Parses one extra-file patch into the store's §6.5 option, refusing
/// an off-vocabulary position with the envelope naming the WIRE field
/// (`extra_local`/`extra_project` — the object the modal's control
/// tree names), the offending position echoed bounded.
fn parse_extra_file(field: &str, patch: &ExtraFilePatch) -> Result<ExtraFile, AppError> {
    let position = match patch.position.as_str() {
        "start" => Position::Start,
        "end" => Position::End,
        _ => {
            return Err(rejected_field(
                field,
                &bounded_echo(&patch.position),
                "the position must be start or end (spec 6.5)",
            ));
        }
    };
    Ok(ExtraFile {
        enabled: patch.enabled,
        position,
    })
}
