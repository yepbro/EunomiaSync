// The mock backend's in-memory state (T-141): everything the mock commands
// read and mutate. Initialized lazily from the fixture generator, reset by
// tests through `resetMockState`.
//
// One row type (`MockProjectRow`) is the single source of truth; the two
// project DTOs the wire actually serves (`ProjectColumnEntry` from
// `projects_list`, `AddedProject` from the two add commands) are mapped from
// it, exactly like the Rust side maps one `projects` row onto both.

import type {
  ActivityEntryWire,
  AddedProjectWire,
  AssignmentRecordWire,
  AssignmentTargetWire,
  BackupCopyRecordWire,
  DbBackupInfo,
  ExclusionEntry,
  ProjectAvailabilityEventWire,
  ProjectColumnEntryWire,
  RevisionRecordWire,
  SettingItem,
  VariantRecordWire,
} from '../contract'
import type { SyncRunSummary_Serialize } from '../../bindings'
import type { ExclusionFixture, ProjectFixture } from '../fixtures/domain'
import {
  BASE_TS_MS,
  generateActivityEntries,
  generateDbBackups,
  generateFixtures,
  parseFixtureProfile,
  type FixtureProfile,
} from '../fixtures/generator'
import { isoUtc } from '../fixtures/rng'
import { DEFAULT_SETTINGS } from '../fixtures/settings'
import { mockEventBus } from './events'

/** The §6.2 CHECK vocabularies, as literal unions for the mock rows. */
export type MockProjectState = 'active' | 'archived'
export type MockProjectScheme = 'A' | 'B' | 'C'
export type MockProjectBaseName = 'AGENTS.md' | 'CLAUDE.md'
export type MockHintPosition = 'start' | 'end'

/** One `projects` row in the mock store (§6.2 columns the UI can reach). */
export type MockProjectRow = {
  id: string
  path: string
  name: string
  label: string
  color: string
  /**
   * §6.2's `description` column (PRJ-008's editable field, T-106):
   * fixture rows seed the column default `''`; `projectUpdate` writes
   * it. Deliberately NOT served by `projectsList` — production's
   * column entry carries no description either (the modal's read
   * surface is a future task's), so tests read the row directly, the
   * §5.7-marking posture.
   */
  description: string
  state: MockProjectState
  addedVia: 'manual' | 'scan'
  sortOrder: number
  scheme: MockProjectScheme
  baseName: MockProjectBaseName
  extraLocalEnabled: boolean
  extraLocalPosition: MockHintPosition
  extraProjectEnabled: boolean
  extraProjectPosition: MockHintPosition
  available: boolean
  unavailableReason: string | null
  dirty: boolean
  /** §5.7 SYS-008: the target file differs from the build (D-137). */
  externalChanged: boolean
  /** §5.7 SYS-009: the target file is gone after a recorded sync. */
  fileDeleted: boolean
  /**
   * §5.7 SYS-010: why the target file is NOT being watched — the
   * closed vocabulary `'gone' | 'limit' | 'other'` (the refusal
   * class, D-155); `null` when nothing was lost. Row state exactly
   * like the §5.7 flags: a UI test edits the row, the carrier maps
   * it onto the wire.
   */
  unwatchedReason: string | null
  lastSyncedAt: string | null
}

/** The whole mutable store the mock commands serve from. */
export type MockState = {
  projects: MockProjectRow[]
  settings: SettingItem[]
  activity: ActivityEntryWire[]
  backups: DbBackupInfo[]
  /** The D-10 screen's rows (§6.2 `project_exclusions`), in wire shape. */
  exclusions: ExclusionEntry[]
  /**
   * The library tree rows (§6.2 `library_items` + the active variants
   * of `instruction_variants`), in `position` order — `libraryGet`
   * (T-083) maps them onto the wire the way `toColumnEntry` maps a
   * project row, and the instruction lifecycle commands (T-328)
   * write them. Dates are derived deterministically from the fixture
   * era's base instant (one second per position).
   */
  library: MockLibraryRow[]
  /**
   * The assignment rows (§6.2 `project_instructions`), the fixture
   * generator's referentially-honest set — `libraryGet`'s label join,
   * the variant commands' delete-flow moves (T-329) and the future
   * assignment commands read and write them. The mutable twin of the
   * readonly fixture shape: the seeding spreads the fixtures in.
   */
  assignments: MockAssignmentRow[]
  /** LIB-026/027 history rows written by the mock library commands. */
  revisions: MockRevisionRow[]
  /**
   * The §6.2 `backup_records` rows (T-125, BKP-004): the project
   * Backups tab's listing. Seeded deterministically per project (the
   * fixture era's stamps, mixed statuses, one run-orphaned `None`
   * link — §6.4's run cleanup leaves those honestly); no mock command
   * writes them yet (the `syncRun` stand-in does not model phase 2),
   * so tests edit the rows directly, exactly like the §5.7 markings.
   */
  backupRecords: MockBackupRecordRow[]
  /** The journal's next autoincrement id (§6.2 `action_log.id`). */
  nextActivityId: number
  /**
   * Whether a mock scan is live (§7.10 `scan_start`/`scan_cancel`): the
   * second-start `E_SYNC_RUNNING` arm's flag, cleared on every exit of
   * the mock walk.
   */
  scanning: boolean
  /** The live mock scan's cancel latch (`scan_cancel` flips it). */
  scanCancelled: boolean
  /**
   * The last COMPLETED scan's reported repository keys (§7.10
   * `scan_commit`'s SEC-003 rule, T-077/D-179): the only paths
   * `scanCommit` admits. Replaced by every completed mock walk; a
   * cancelled walk leaves it untouched (ERR-002).
   */
  scanFindings: string[]
  /**
   * The `syncRun` stand-in's memory of its last completed run (T-121):
   * the answer's summary payload remembered verbatim plus the
   * run-level §10 code (always `null` here — the stand-in has no
   * abort paths, its verdicts are `success`/`partial`/`failed` over
   * per-project rows that carry their own codes). A one-run memory,
   * not a history: a later run replaces it, a reset clears it, and
   * the seeded fixture carries none (the mock writes no run rows —
   * `syncRun`'s own documented limit, so the two sync reads serve
   * from this memory instead of mirroring the store).
   */
  lastSyncRun: { summary: SyncRunSummary_Serialize; errorCode: string | null } | null
  /**
   * The `finished_at` of the last mock run with the `success` verdict
   * (MAIN-027's rule mirrored): moved only by a success, never by a
   * `partial`/`failed` run — the stand-in's own approximation of the
   * derived "newest success run" read (no run history exists to
   * derive from).
   */
  lastSyncSuccessAt: string | null
  /**
   * The close-confirmation flow's one-way latch (T-123/D-219): `true`
   * once a mock `syncConfirmExit` call was accepted. The stand-in's
   * own memory of the confirm — nothing drives it from the mock side
   * (the mock has no window machinery to intercept a close), and the
   * EXIT itself is not model-able in a browser mock (no process to
   * end — the documented limit `syncConfirmExit` carries); what the
   * bit gives the dialog's future tests is the "the confirm landed"
   * fact. A reset clears it (a fresh world is a fresh session, the
   * latch's own per-process posture).
   */
  exitConfirmed: boolean
}

/** One `library_items` row in the mock store (§6.2 columns the UI can
 * reach): a heading or an instruction with its active variants. */
export type MockLibraryRow =
  | {
      kind: 'heading'
      id: string
      position: number
      headingLevel: 2 | 3 | 4 | 5 | 6
      title: string
      state: 'active'
      createdAt: string
      updatedAt: string
    }
  | {
      kind: 'instruction'
      id: string
      position: number
      title: string
      state: 'active' | 'hidden'
      /** The instruction's variants (§6.2 `instruction_variants`): the
       * seeded fixture set is all-active, the variant commands' own
       * soft deletes are the one source of `deleted` rows —
       * `libraryGet` serves the active subset (DB-012). */
      variants: MockVariantRow[]
      createdAt: string
      updatedAt: string
    }

/** One variant of a mock instruction row (the `VariantFixture` shape,
 * mutable — a UI test edits the row, exactly like the project rows).
 * The variant commands (T-329) extend the fixture shape with the
 * `instruction_variants` columns their wire DTO reaches: the §6.4
 * soft-delete state (the fixture set seeds none — the commands'
 * own deletes are the only source of `deleted` rows), and the DB-002
 * dates (one second per position, plus the variant's own index —
 * deterministic within a reset, distinct per row). */
export type MockVariantRow = {
  id: string
  name: string
  content: string
  isMain: boolean
  state: 'active' | 'deleted'
  createdAt: string
  updatedAt: string
  deletedAt: string | null
}

/** One `project_instructions` row in the mock store (§6.2, D-02/DB-012):
 * one of the three target shapes per project/instruction pair — a
 * variant, replace text, or MAIN-034's extend pair (base variant +
 * supplement). "Neither" stays unrepresentable in TypeScript too. */
export type MockAssignmentRow = {
  projectId: string
  instructionId: string
  target: AssignmentTargetWire
  createdAt: string
  updatedAt: string
}

/** One stored history row; the wire shape is already the table's shape. */
export type MockRevisionRow = RevisionRecordWire

/**
 * One `backup_records` row in the mock store (§6.2 columns the
 * `backups_list` wire reaches): the mutable twin of the store's
 * `BackupRecord` — the wire shape plus the scoping `projectId` the
 * wire deliberately omits (D-193: the project is the scope, not
 * per-row data). Ids and run ids derive from the project id (the
 * fixture world has no runs table — the wire row is the whole claim),
 * and the sizes are plain fixture numbers.
 */
export type MockBackupRecordRow = BackupCopyRecordWire & { projectId: string }

/** Maps a mock assignment row to the generated command DTO. */
export function toAssignmentRecord(row: MockAssignmentRow): AssignmentRecordWire {
  return {
    project_id: row.projectId,
    instruction_id: row.instructionId,
    target: row.target,
    created_at: row.createdAt,
    updated_at: row.updatedAt,
  }
}

/**
 * Maps a store row to the `projects_list` wire shape (MAIN-028 fields).
 * The verdict halves are row-decidable like the real read's functions
 * (one truth with the backend): `dirty` is DB-014's archived arm
 * (D-128), and the §5.7 watch pair + the SYS-010 notice read `false/
 * false/null` for an ARCHIVED row (SYS-007's watch set — round 2's
 * P2-1) whatever its own flags still say, so a UI test that archives
 * a flagged row sees exactly what production serves. The UNAVAILABLE
 * half of production's pair is deliberately NOT mirrored here: the
 * mock's stand-in rows carry their flags verbatim for the unavailable
 * case (a documented pre-existing approximation, syncRun's own
 * skipped-row pin) — aligning it is its own follow-up, not this
 * command's change.
 */
export function toColumnEntry(row: MockProjectRow): ProjectColumnEntryWire {
  const archived = row.state === 'archived'
  return {
    id: row.id,
    name: row.name,
    label: row.label,
    color: row.color,
    state: row.state,
    available: row.available,
    unavailable_reason: row.unavailableReason,
    dirty: archived ? false : row.dirty,
    external_changed: archived ? false : row.externalChanged,
    file_deleted: archived ? false : row.fileDeleted,
    unwatched_reason: archived ? null : row.unwatchedReason,
    last_synced_at: row.lastSyncedAt,
  }
}

/** Maps a store row to the `"added"` payload of `project_add_manual`. */
export function toAddedProject(row: MockProjectRow): AddedProjectWire {
  return {
    id: row.id,
    path: row.path,
    name: row.name,
    label: row.label,
    color: row.color,
    added_via: row.addedVia,
    sort_order: row.sortOrder,
    scheme: row.scheme,
    base_name: row.baseName,
    extra_local_enabled: row.extraLocalEnabled,
    extra_local_position: row.extraLocalPosition,
    extra_project_enabled: row.extraProjectEnabled,
    extra_project_position: row.extraProjectPosition,
    // The mock has no filesystem, so the write-access probe always answers
    // "writable" (`null`); the warning arms stay reachable in tests by
    // editing the row directly.
    write_access_warning: null,
  }
}

/** Maps an instruction row's variant to the `VariantRecord` wire shape
 * (the §7.10 variant commands' answer; snake_case fields like the other
 * DTOs — the instruction id comes from the owning row, the union
 * model's parent link). */
export function toVariantRecord(
  instruction: MockLibraryRow & { kind: 'instruction' },
  variant: MockVariantRow,
): VariantRecordWire {
  return {
    id: variant.id,
    instruction_id: instruction.id,
    name: variant.name,
    content: variant.content,
    is_main: variant.isMain,
    state: variant.state,
    created_at: variant.createdAt,
    updated_at: variant.updatedAt,
    deleted_at: variant.deletedAt,
  }
}

// ---------------------------------------------------------------------------
// The §7.10 `state://project-availability` source (T-246): the mock twin
// of the backend's check pass (`availability::recheck_and_emit`, D-112).
// The bus lives in `./events`; this is the seam that feeds it — the mock
// has no filesystem to probe, so tests and fixtures state the verdicts
// and the seam computes the diff exactly as the backend's session cache
// does: against a per-session verdict cache (not the rows), where a
// project never probed this session diffs against the implicit
// `(available, no reason)` baseline. The rows are the served state alone
// (`projects_list` maps from them). Known limit, deliberate: the backend
// prunes a project's cache entry when it leaves the active list, while
// the mock's cache is replaced only by `resetMockState` — the
// project-row-mutating mock commands (`projectAddManual`/`scanCommit`
// append rows, `projectsReorder` renumbers `sort_order`, `syncRun`
// stamps and clears `dirty`/the §5.7 markings) never touch the cache,
// so a mutated row keeps its recorded verdict until an explicit probe
// of an active row or a reset.
// ---------------------------------------------------------------------------

/** The D-084 unavailable-reason spellings (single-sourced in the backend's
 * `Verdict::probe`; mirrored here so a mock verdict cannot invent one). */
export type MockUnavailableReason = 'missing' | 'not_a_directory' | 'not_readable' | 'no_git_entry'

/**
 * One probe verdict for [`applyMockAvailability`]. An available folder
 * carries no reason — the union makes "available with a reason"
 * unrepresentable, which maps onto the wire's `unavailable_reason: null`.
 */
export type MockAvailabilityVerdict =
  | { projectId: string; available: true }
  | { projectId: string; available: false; unavailableReason: MockUnavailableReason }

/** The verdict pair the session cache records for one probed project. */
type RecordedVerdict = { available: boolean; reason: string | null }

/**
 * The implicit baseline for a project never probed this session (D-112):
 * `(available, no reason)` — a healthy first check emits nothing, and a
 * project first probed while its folder is gone IS a change.
 */
const NEVER_PROBED: RecordedVerdict = { available: true, reason: null }

/** The session verdict cache — the mock twin of the backend's
 * `AvailabilityState`; replaced wholesale by `resetMockState`. */
let availabilityCache = new Map<string, RecordedVerdict>()

/**
 * Applies new availability verdicts the way the backend's check pass does
 * (D-112: `availability::recheck_and_emit`) and broadcasts the §7.10
 * `state://project-availability` event over [`mockEventBus`] — returning
 * the emitted diff, or `null` when nothing changed (the event is never
 * emitted with an empty diff).
 *
 * Semantics, mirroring the backend's session diff cache:
 * - the diff is against the SESSION cache, not the row: a project never
 *   probed this session (since the last `resetMockState`) counts as
 *   `(available, no reason)` — D-112's baseline rule, so a healthy first
 *   check is silent while a first probe of a gone folder is a change;
 * - the pass is all-or-nothing: an unknown project id throws before any
 *   verdict is applied, cached, or emitted (a half-applied pass that
 *   emits nothing is exactly the divergence the backend's at-least-once
 *   rule exists to prevent);
 * - a project listed twice keeps its LAST verdict, and a mid-pass revert
 *   leaves no change entry — the diff is the pass's net effect, the one
 *   comparison the backend makes against its cache;
 * - changes are contradictions only: a flip in either direction, or a
 *   new reason on a project that stays unavailable;
 * - changes are ordered by `sort_order` (probe order) and emitted as ONE
 *   event per call, like one backend pass;
 * - only ACTIVE rows are probed (the backend's pass reads the active
 *   list): a verdict for an archived project is skipped, not recorded;
 * - verdicts are recorded on the cache and the rows, so `projects_list`
 *   reads them back.
 */
export function applyMockAvailability(
  verdicts: readonly MockAvailabilityVerdict[],
): ProjectAvailabilityEventWire | null {
  const rows = mockState().projects
  const byId = new Map(rows.map((row) => [row.id, row]))
  // Phase 1 — validate the whole pass before any side effect: a bad id
  // throws with the store and the cache untouched.
  for (const verdict of verdicts) {
    if (!byId.has(verdict.projectId)) {
      throw new Error(`applyMockAvailability: unknown project id ${verdict.projectId}`)
    }
  }
  // Phase 2 — the pass's net verdict per active project (last verdict
  // per id wins; archived rows are not probed).
  const net = new Map<string, MockAvailabilityVerdict>()
  for (const verdict of verdicts) {
    if (byId.get(verdict.projectId)!.state === 'active') net.set(verdict.projectId, verdict)
  }
  // Phase 3 — diff against the session cache, then record: every probed
  // verdict is recorded (a confirming probe updates the cache like the
  // backend's), and only contradictions become change entries.
  const changedRows: MockProjectRow[] = []
  for (const [projectId, verdict] of net) {
    const row = byId.get(projectId)!
    const recorded = availabilityCache.get(projectId) ?? NEVER_PROBED
    const isChange =
      verdict.available !== recorded.available ||
      (!verdict.available && verdict.unavailableReason !== recorded.reason)
    availabilityCache.set(projectId, {
      available: verdict.available,
      reason: verdict.available ? null : verdict.unavailableReason,
    })
    row.available = verdict.available
    row.unavailableReason = verdict.available ? null : verdict.unavailableReason
    if (isChange) changedRows.push(row)
  }
  if (changedRows.length === 0) return null
  changedRows.sort((a, b) => a.sortOrder - b.sortOrder)
  const event: ProjectAvailabilityEventWire = {
    changes: changedRows.map((row) => ({
      project_id: row.id,
      available: row.available,
      unavailable_reason: row.unavailableReason,
    })),
  }
  mockEventBus.emit('state://project-availability', event)
  return event
}

/** Default write settings a new project copies from `default.*` (PRJ-012). */
function defaultsFromSettings(settings: SettingItem[]): {
  scheme: MockProjectScheme
  baseName: MockProjectBaseName
  extraLocalEnabled: boolean
  extraLocalPosition: MockHintPosition
  extraProjectEnabled: boolean
  extraProjectPosition: MockHintPosition
} {
  const valueOf = (key: string): unknown =>
    JSON.parse(settings.find((s) => s.key === key)?.value ?? 'null')
  const extraLocal = valueOf('default.extra_local') as {
    enabled: boolean
    position: MockHintPosition
  }
  const extraProject = valueOf('default.extra_project') as {
    enabled: boolean
    position: MockHintPosition
  }
  return {
    scheme: valueOf('default.scheme') as MockProjectScheme,
    baseName: valueOf('default.base_name') as MockProjectBaseName,
    extraLocalEnabled: extraLocal.enabled,
    extraLocalPosition: extraLocal.position,
    extraProjectEnabled: extraProject.enabled,
    extraProjectPosition: extraProject.position,
  }
}

/** Builds the initial rows from a fixture bundle. */
function rowsFromFixtures(
  bundle: ReturnType<typeof generateFixtures>,
  settings: SettingItem[],
): MockProjectRow[] {
  const defaults = defaultsFromSettings(settings)
  return bundle.projects.map((project: ProjectFixture, i) => ({
    id: project.id,
    path: `/home/dev/src/${project.name.toLowerCase().replace(/[^a-z0-9]+/g, '-')}`,
    name: project.name,
    label: project.label,
    color: project.color,
    description: '',
    state: project.state === 'archived' ? 'archived' : 'active',
    addedVia: 'scan',
    sortOrder: i + 1,
    scheme: defaults.scheme,
    baseName: defaults.baseName,
    extraLocalEnabled: defaults.extraLocalEnabled,
    extraLocalPosition: defaults.extraLocalPosition,
    extraProjectEnabled: defaults.extraProjectEnabled,
    extraProjectPosition: defaults.extraProjectPosition,
    available: project.available,
    unavailableReason: project.unavailable_reason,
    dirty: project.dirty,
    externalChanged: project.external_changed,
    fileDeleted: project.file_deleted,
    unwatchedReason: project.unwatched_reason,
    lastSyncedAt: project.last_synced_at,
  }))
}

/** Maps a fixture row to the `exclusions_list` wire shape. */
function toExclusionEntry(row: ExclusionFixture): ExclusionEntry {
  return {
    id: row.id,
    path: row.path,
    path_key: row.pathKey,
    reason: row.reason,
    created_at: row.createdAt,
  }
}

/** Maps a mock backup-record row to the `backups_list` wire shape
 * (the store row keeps the scoping key the wire deliberately omits —
 * D-193's ruling: the project is the scope, not per-row data). */
export function toBackupCopyRecord(row: MockBackupRecordRow): BackupCopyRecordWire {
  return {
    id: row.id,
    sync_run_id: row.sync_run_id,
    source_path: row.source_path,
    backup_path: row.backup_path,
    size_bytes: row.size_bytes,
    status: row.status,
    created_at: row.created_at,
  }
}

/**
 * Builds the deterministic per-project backup-record rows (T-125,
 * BKP-004): one row per §6.2 CHECK status on a fixed timeline, oldest
 * to newest — the run-orphaned `discarded` row (§6.4's run cleanup
 * swept its run, hence the `null` link), the `missing` copy, the
 * limit-`pruned` row, the live `ok` copy. Stamps sit in the fixture
 * era (one second per row, offset past every other table's slots),
 * ids and run ids derive from the project id, and the sizes are plain
 * distinct fixture numbers. Every project gets its history — the
 * read is state-agnostic (DB-014), archived rows included.
 */
function backupRecordsFromProjects(projects: readonly MockProjectRow[]): MockBackupRecordRow[] {
  const rows: MockBackupRecordRow[] = []
  projects.forEach((project, projectIndex) => {
    const slug = project.path.split('/').pop() ?? 'project'
    const statuses: BackupCopyRecordWire['status'][] = ['discarded', 'missing', 'pruned', 'ok']
    statuses.forEach((status, n) => {
      const stamp = isoUtc(BASE_TS_MS + (300 + projectIndex * 4 + n) * 1000)
      rows.push({
        id: `${project.id}-bak-${n}`,
        // The oldest row's run was swept by the §6.4 cleanup: the FK's
        // SET NULL leaves the honest `null` link.
        sync_run_id: n === 0 ? null : `${project.id}-run-${n}`,
        source_path: `${project.path.replace(/\/$/, '')}/AGENTS.md`,
        // The §5.5 layout name with its compact UTC stamp derived from
        // the row's own DB-002 stamp (BKP-001's `YYYYMMDDTHHMMSSmmmZ`
        // shape), so the name and the record agree about when the copy
        // was made — the store module's invariant, mock twin.
        backup_path: `backups/${project.id}__${slug}/AGENTS.md.${compactStamp(stamp)}.${`${n}`.padStart(8, '0')}.bak`,
        size_bytes: 64 * (n + 1),
        status,
        created_at: stamp,
        projectId: project.id,
      })
    })
  })
  return rows
}

/**
 * The §5.5 backup name's compact UTC stamp (BKP-001:
 * `YYYYMMDDTHHMMSSmmmZ`), derived from the row's DB-002 stamp —
 * the mock twin of the core builder's time component, so the
 * name and the record never disagree about the copy's instant.
 */
function compactStamp(stamp: string): string {
  return stamp.replace(/[-:.]/g, '')
}

let state: MockState | undefined

/**
 * The mock store. Lazily initialized on first access (set L is expensive;
 * a test that never touches IPC data must not pay for it) from the profile
 * selected by `VITE_IPC_FIXTURES` (set S by default).
 */
export function mockState(): MockState {
  if (!state) {
    state = resetMockState(parseFixtureProfile(import.meta.env.VITE_IPC_FIXTURES))
  }
  return state
}

/**
 * Replaces the store with fresh data for a profile. Tests call this in
 * `beforeEach` to pin their own small worlds.
 */
export function resetMockState(profile: FixtureProfile): MockState {
  const bundle = generateFixtures(profile)
  const settings = DEFAULT_SETTINGS.map((s) => ({ ...s }))
  const projects = rowsFromFixtures(bundle, settings)
  const next: MockState = {
    projects,
    settings,
    activity: generateActivityEntries(bundle.projects, profile),
    backups: generateDbBackups(profile),
    exclusions: bundle.exclusions.map(toExclusionEntry),
    library: bundle.library.map((item) =>
      item.kind === 'heading'
        ? {
            kind: 'heading' as const,
            id: item.id,
            position: item.position,
            headingLevel: item.headingLevel,
            title: item.title,
            state: 'active' as const,
            // DB-002 dates, one second per position: deterministic
            // within a reset, distinct per row.
            createdAt: isoUtc(BASE_TS_MS + item.position * 1000),
            updatedAt: isoUtc(BASE_TS_MS + item.position * 1000),
          }
        : {
            kind: 'instruction' as const,
            id: item.id,
            position: item.position,
            title: item.title,
            state: item.state,
            variants: item.variants.map((variant, index) => ({
              ...variant,
              state: 'active' as const,
              deletedAt: null,
              // The variant's own dates: one second per position plus
              // its index — distinct per row, inside the same
              // deterministic era.
              createdAt: isoUtc(BASE_TS_MS + (item.position + 1) * 1000 + index),
              updatedAt: isoUtc(BASE_TS_MS + (item.position + 1) * 1000 + index),
            })),
            createdAt: isoUtc(BASE_TS_MS + item.position * 1000),
            updatedAt: isoUtc(BASE_TS_MS + item.position * 1000),
          },
    ),
    assignments: bundle.assignments.map((assignment, index) => {
      const stamp = isoUtc(BASE_TS_MS + index * 1000)
      return {
        projectId: assignment.projectId,
        instructionId: assignment.instructionId,
        target: { kind: 'variant' as const, value: assignment.variantId },
        createdAt: stamp,
        updatedAt: stamp,
      }
    }),
    revisions: [],
    backupRecords: backupRecordsFromProjects(projects),
    nextActivityId: 1_000_000,
    scanning: false,
    scanCancelled: false,
    // A fresh world is a fresh session: no scan completed yet, so the
    // commit admits nothing.
    scanFindings: [],
    // No mock run completed yet: the sync reads answer the cold-start
    // facts (`noRuns`, no success time — the seeded world has no run
    // rows, and the stand-in's memory starts empty with it).
    lastSyncRun: null,
    lastSyncSuccessAt: null,
    // No close was confirmed in a fresh world (the latch's per-process
    // posture mirrored: the fresh session starts unconfirmed).
    exitConfirmed: false,
  }
  // A fresh world is a fresh session: no project has been probed yet, so
  // the availability cache restarts from the D-112 baseline.
  state = next
  availabilityCache = new Map()
  return next
}

// Runtime-added rows need ids the fixtures never generated. UUIDv7-shaped
// from the wall clock plus a monotonic counter: uniqueness by construction
// within a session, without pulling `crypto` in (and without touching the
// seeded PRNG, which must stay fixture-deterministic).
let runtimeCounter = 0
/** A fresh UUIDv7-shaped id for a row created at runtime (not a fixture). */
export function freshRowId(nowMs: number = Date.now()): string {
  runtimeCounter += 1
  const safeNow = Math.max(0, Math.floor(nowMs))
  const tsHi = Math.floor(safeNow / 2 ** 32)
  const tsLo = safeNow % 2 ** 32
  const ts = (tsHi.toString(16).padStart(4, '0') + tsLo.toString(16).padStart(8, '0')).slice(-12)
  const ctr = runtimeCounter.toString(16).padStart(8, '0') + '0000000000'
  const hex = ts + '7' + ctr.slice(0, 3) + 'a' + ctr.slice(3)
  return [
    hex.slice(0, 8),
    hex.slice(8, 12),
    hex.slice(12, 16),
    hex.slice(16, 20),
    hex.slice(20, 32),
  ].join('-')
}

export { defaultsFromSettings }
