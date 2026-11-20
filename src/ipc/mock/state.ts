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
