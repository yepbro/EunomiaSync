// The mock backend's in-memory state (T-141): everything the mock commands
// read and mutate. Initialized lazily from the fixture generator, reset by
// tests through `resetMockState`.
//
// One row type (`MockProjectRow`) is the single source of truth; the two
// project DTOs the wire actually serves (`ProjectColumnEntry` from
// `projects_list`, `AddedProject` from `project_add_manual`) are mapped from
// it, exactly like the Rust side maps one `projects` row onto both.

import type {
  ActivityEntryWire,
  AddedProjectWire,
  DbBackupInfo,
  ExclusionEntry,
  ProjectAvailabilityEventWire,
  ProjectColumnEntryWire,
  SettingItem,
} from '../contract'
import type { ExclusionFixture, ProjectFixture } from '../fixtures/domain'
import {
  generateActivityEntries,
  generateDbBackups,
  generateFixtures,
  parseFixtureProfile,
  type FixtureProfile,
} from '../fixtures/generator'
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
  /** The journal's next autoincrement id (§6.2 `action_log.id`). */
  nextActivityId: number
}

/** Maps a store row to the `projects_list` wire shape (MAIN-028 fields). */
export function toColumnEntry(row: MockProjectRow): ProjectColumnEntryWire {
  return {
    id: row.id,
    name: row.name,
    label: row.label,
    color: row.color,
    state: row.state,
    available: row.available,
    unavailable_reason: row.unavailableReason,
    dirty: row.dirty,
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
// the mock's cache is replaced only by `resetMockState` — no mock command
// mutates project state yet, and a row archived/restored by direct
// editing keeps its recorded verdict.
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
 *
 * Note: the store currently serves only what the existing commands read
 * (projects, settings, activity, backups); the generated library and
 * assignment fixtures are built and held for the Library/Assignments
 * command tasks to wire in (set L's 100 000 assignments cost ~100 ms to
 * build — acceptable inside the §12.2 startup budget, and skipped entirely
 * by every profile that does not ask for them).
 */
export function resetMockState(profile: FixtureProfile): MockState {
  const bundle = generateFixtures(profile)
  const settings = DEFAULT_SETTINGS.map((s) => ({ ...s }))
  state = {
    projects: rowsFromFixtures(bundle, settings),
    settings,
    activity: generateActivityEntries(bundle.projects, profile),
    backups: generateDbBackups(profile),
    exclusions: bundle.exclusions.map(toExclusionEntry),
    nextActivityId: 1_000_000,
  }
  // A fresh world is a fresh session: no project has been probed yet, so
  // the availability cache restarts from the D-112 baseline.
  availabilityCache = new Map()
  return state
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
