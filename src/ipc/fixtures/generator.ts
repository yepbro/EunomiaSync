// The deterministic fixture generator (T-141): one parameterized profile
// in, one coherent domain snapshot out — projects (the T-072 `projects_list`
// shape), library items with variants, assignments, the activity journal
// and the db-backups list.
//
// Rules the whole module is built on:
// - Determinism: the same profile (seed included) produces structurally
//   identical data. No `Math.random`, no `Date.now` — only the seeded PRNG
//   and a fixed base timestamp, so a scroll failure against set L on one
//   machine reproduces exactly on another.
// - Scale is a parameter. Presets follow spec §12.2: set M (50 projects,
//   200 instructions) and set L (500 projects, 1000 instructions, 100
//   headings, 100 000 assignments, 3 variants per instruction) — the upper
//   limit the performance targets are measured against. Tests use small
//   custom profiles or set S; M and L are opt-in (L is not generated in the
//   unit suite).
// - Referential honesty: every assignment names a real project, a real
//   active instruction and one of that instruction's variants; ids are
//   unique. Violations throw at generation time — a broken fixture must
//   never reach a test as a silent wrong fact.

import type { ActivityEntryWire, DbBackupInfo } from '../contract'
import { createRng, isoUtc, uuidv7From, type Rng } from './rng'
import { SETTING_KEYS } from './settings'
import { defaultLabel, pathKeyOf, type ExclusionFixture } from './domain'
import type {
  AssignmentFixture,
  FixtureBundle,
  HeadingFixture,
  InstructionFixture,
  LibraryItemFixture,
  ProjectFixture,
  VariantFixture,
} from './domain'

/** Everything that sizes a fixture set. All members required and explicit. */
export type FixtureProfile = {
  /** PRNG seed; the same seed plus counts reproduce the same data. */
  readonly seed: number
  /** Total project rows (active + archived). */
  readonly projects: number
  /** Instruction rows (heading rows are extra, per `headings`). */
  readonly instructions: number
  /** Heading rows. */
  readonly headings: number
  /**
   * `project_instructions` rows. Must be ≤ projects × active instructions;
   * larger values throw instead of silently emitting duplicate pairs.
   */
  readonly assignments: number
  /** How many of the LAST projects are archived (§6.4 keeps their order). */
  readonly archivedProjects: number
  /** Variants per instruction; the first is the `Main` variant (§6.9). */
  readonly variantsPerInstruction: number
  /** Excluded-path rows for the D-10 screen (§6.2 `project_exclusions`). */
  readonly exclusions: number
}

/** Set S (spec §15.1 smoke scale): tiny, for unit tests and default dev. */
export const FIXTURE_PROFILE_S: FixtureProfile = {
  seed: 141,
  projects: 3,
  instructions: 12,
  headings: 1,
  assignments: 6,
  archivedProjects: 1,
  variantsPerInstruction: 1,
  exclusions: 2,
}

/** Set M (spec §12.2): 50 projects, 200 instructions — the mid-size set. */
export const FIXTURE_PROFILE_M: FixtureProfile = {
  seed: 142,
  projects: 50,
  instructions: 200,
  headings: 20,
  assignments: 2500,
  archivedProjects: 5,
  variantsPerInstruction: 2,
  exclusions: 2,
}

/** Set L (spec §12.2): the upper-limit scroll set. Opt-in only. */
export const FIXTURE_PROFILE_L: FixtureProfile = {
  seed: 143,
  projects: 500,
  instructions: 1000,
  headings: 100,
  assignments: 100000,
  archivedProjects: 20,
  variantsPerInstruction: 3,
  exclusions: 2,
}

/**
 * Maps the `VITE_IPC_FIXTURES` dev knob to a profile: `s`, `m` or `l`
 * (case-insensitive); anything else — unset included — is set S. A dev-only
 * convenience, not a contract: the README documents the values.
 */
export function parseFixtureProfile(raw: string | undefined): FixtureProfile {
  const key = (raw ?? 's').trim().toLowerCase()
  if (key === 'm') return FIXTURE_PROFILE_M
  if (key === 'l') return FIXTURE_PROFILE_L
  return FIXTURE_PROFILE_S
}

/** The DB-040 fixed palette, in spec order. */
export const PALETTE: readonly string[] = [
  'red',
  'orange',
  'amber',
  'lime',
  'green',
  'teal',
  'cyan',
  'blue',
  'indigo',
  'violet',
  'pink',
  'slate',
]

/** The D-084 unavailable reasons, in the wire's stored spellings. */
const UNAVAILABLE_REASONS: readonly string[] = [
  'missing',
  'not_a_directory',
  'not_readable',
  'no_git_entry',
]

const NAME_WORDS: readonly string[] = [
  'api-gateway',
  'task-board',
  'sync-engine',
  'docs-site',
  'cli-tool',
  'web-client',
  'parser-core',
  'mobile-app',
  'infra-plan',
  'design-kit',
  'auth-service',
  'data-lake',
]

const HEADING_WORDS: readonly string[] = [
  'General',
  'Workflow',
  'Editor',
  'Project rules',
  'Testing',
  'Release',
]

const INSTRUCTION_WORDS: readonly string[] = [
  'Use pnpm',
  'Keep functions small',
  'Write tests first',
  'Follow the style guide',
  'Update the changelog',
  'Run the linter',
  'Sign your commits',
  'Document public APIs',
  'Respect the schema',
  'Prefer explicit types',
]

const ACTION_WORDS: readonly string[] = [
  'project.added',
  'project.archived',
  'settings.changed',
  'sync.run_finished',
  'sync.project_result',
]

/**
 * The closed `action_log.action` vocabulary in its stored spellings — a
 * deliberate mirror of `Action::as_str` in
 * `crates/eunomia-store/src/action_log.rs` (22 members; the generator
 * samples a subset, the mock's `activity_list` validates against the full
 * set). When a task adds a member, this list grows with it.
 */
export const ACTION_MEMBERS: readonly string[] = [
  'project.added',
  'project.changed',
  'project.archived',
  'project.restored',
  'settings.changed',
  'instruction.created',
  'instruction.changed',
  'instruction.hidden',
  'instruction.restored',
  'variant.created',
  'variant.changed',
  'variant.deleted',
  'variant.restored',
  'assignment.changed',
  'heading.created',
  'heading.changed',
  'heading.deleted',
  'sync.run_finished',
  'sync.project_result',
  'backup.created',
  'error.reported',
  'exclusion.removed',
]

const EXCLUSION_WORDS: readonly string[] = [
  'old-prototype',
  'vendored-sdk',
  'legacy-mono',
  'scratch-pad',
]

/** All fixture timestamps are offsets from this fixed instant. */
/** The fixture era's base instant (2026-11-20T00:00:00Z): every generated
 * timestamp and UUIDv7 id derives from it, keeping worlds
 * date-deterministic. Shared with `state.ts`, whose library rows derive
 * their DB-002 dates from the same base. */
export const BASE_TS_MS = Date.UTC(2026, 10, 20)
const DAY_MS = 24 * 60 * 60 * 1000

function projectName(rng: Rng, index: number): string {
  // The index suffix keeps names unique; DB-041 allows duplicates, but a
  // fixture set where two columns are indistinguishable hides bugs.
  return `${rng.pick(NAME_WORDS)} ${index}`
}

/**
 * Generates the domain snapshot for a profile. Deterministic: two calls
 * with the same profile return deep-equal bundles.
 */
export function generateFixtures(profile: FixtureProfile): FixtureBundle {
  if (profile.projects < 0 || profile.instructions < 0) {
    throw new Error('profile counts must be non-negative')
  }
  if (profile.archivedProjects > profile.projects) {
    throw new Error('archivedProjects cannot exceed projects')
  }
  if (profile.variantsPerInstruction < 1) {
    throw new Error('variantsPerInstruction must be at least 1')
  }
  const rng = createRng(profile.seed)

  // --- projects -----------------------------------------------------------
  const projectIds: string[] = []
  const projects: ProjectFixture[] = []
  const seenProjectIds = new Set<string>()
  for (let i = 0; i < profile.projects; i++) {
    const id = uuidv7From(rng, BASE_TS_MS + i)
    if (seenProjectIds.has(id)) {
      throw new Error(`duplicate project id generated at index ${i}`)
    }
    seenProjectIds.add(id)
    projectIds.push(id)
    const name = projectName(rng, i)
    // Every 11th project is the D-09 case: moved, deleted or unreadable —
    // the icon-and-text marking the matrix must render.
    const unavailable = i % 11 === 10
    const available = !unavailable
    const synced = available && rng.bool()
    projects.push({
      id,
      name,
      label: defaultLabel(name),
      color: PALETTE[i % PALETTE.length],
      state: i >= profile.projects - profile.archivedProjects ? 'archived' : 'active',
      available,
      unavailable_reason: unavailable ? UNAVAILABLE_REASONS[i % UNAVAILABLE_REASONS.length] : null,
      // Placeholder: assignments decide dirtiness in the second pass.
      dirty: false,
      // Placeholders: the watch pass decides both §5.7 markings and
      // the SYS-010 lost-watch notice in the final pass.
      external_changed: false,
      file_deleted: false,
      unwatched_reason: null,
      last_synced_at: synced ? isoUtc(BASE_TS_MS - rng.int(1, 30) * DAY_MS) : null,
    })
  }

  // --- library ------------------------------------------------------------
  // One ordered list: headings interleave instructions (a heading roughly
  // every ninth row) so position handling has real data to preserve.
  const totalItems = profile.instructions + profile.headings
  const library: LibraryItemFixture[] = []
  const instructionIds: string[] = []
  const activeInstructionIds: string[] = []
  const seenItemIds = new Set<string>()
  let instructionsLeft = profile.instructions
  let headingsLeft = profile.headings
  for (let i = 0; i < totalItems; i++) {
    const id = uuidv7From(rng, BASE_TS_MS + profile.projects + i)
    if (seenItemIds.has(id)) {
      throw new Error(`duplicate library id generated at index ${i}`)
    }
    seenItemIds.add(id)
    const position = i + 1
    // Headings land on every 9th row while any remain; the tail fills with
    // whichever kind is left.
    const headingHere = headingsLeft > 0 && (i % 9 === 4 || instructionsLeft === 0)
    if (headingHere) {
      headingsLeft--
      const heading: HeadingFixture = {
        kind: 'heading',
        id,
        position,
        headingLevel: rng.pick([2, 3, 4, 5, 6] as const),
        title: `${rng.pick(HEADING_WORDS)} ${headingsLeft}`,
        state: 'active',
      }
      library.push(heading)
      continue
    }
    if (instructionsLeft <= 0) continue
    instructionsLeft--
    // Every 13th instruction is hidden (§6.2 `state` vocabulary) — the
    // library's soft-hidden rows (set S has 13 library items, so the
    // default dev world includes exactly one).
    const state = i % 13 === 12 ? 'hidden' : 'active'
    const variants: VariantFixture[] = []
    const seenVariantIds = new Set<string>()
    for (let v = 0; v < profile.variantsPerInstruction; v++) {
      const vid = uuidv7From(rng, BASE_TS_MS + profile.projects + totalItems + i * 10 + v)
      if (seenVariantIds.has(vid)) {
        throw new Error(`duplicate variant id generated at item ${i}, variant ${v}`)
      }
      seenVariantIds.add(vid)
      variants.push({
        id: vid,
        name: v === 0 ? 'Main' : `Variant ${v}`,
        content:
          v === 0
            ? `${rng.pick(INSTRUCTION_WORDS)}.\n\nRun the checks before handing over.`
            : `${rng.pick(INSTRUCTION_WORDS)} (project override).`,
        isMain: v === 0,
      })
    }
    const instruction: InstructionFixture = {
      kind: 'instruction',
      id,
      position,
      title: `${rng.pick(INSTRUCTION_WORDS)} ${instructionIds.length}`,
      state,
      variants,
    }
    library.push(instruction)
    instructionIds.push(id)
    if (state === 'active') activeInstructionIds.push(id)
  }

  // --- assignments ---------------------------------------------------------
  // Pair (i % P, floor(i / P) % I): unique pairs while i < P × I, spread
  // evenly across the project columns — what a scroll test needs.
  const maxAssignments = profile.projects * activeInstructionIds.length
  if (profile.assignments > maxAssignments) {
    throw new Error(
      `profile.assignments (${profile.assignments}) exceeds the unique pair ` +
        `space (projects ${profile.projects} × active instructions ` +
        `${activeInstructionIds.length} = ${maxAssignments})`,
    )
  }
  const assignments: AssignmentFixture[] = []
  const seenPairs = new Set<string>()
  const dirtyProjects = new Set<number>()
  // Index map instead of a linear scan: set L builds 100 000 assignments
  // against 1000+ instructions, and O(n·m) generation would dominate the
  // very startup it is supposed to measure.
  const instructionById = new Map<string, InstructionFixture>()
  for (const item of library) {
    if (item.kind === 'instruction') instructionById.set(item.id, item)
  }
  for (let i = 0; i < profile.assignments; i++) {
    const p = i % profile.projects
    const instructionIdx = Math.floor(i / profile.projects) % activeInstructionIds.length
    const instruction = instructionById.get(activeInstructionIds[instructionIdx])
    if (!instruction) {
      throw new Error(`active instruction ${instructionIdx} disappeared mid-generation`)
    }
    const pairKey = `${p}:${instruction.id}`
    if (seenPairs.has(pairKey)) {
      throw new Error(`duplicate assignment pair generated: ${pairKey}`)
    }
    seenPairs.add(pairKey)
    // Every 3rd assignment picks a non-main variant when one exists — the
    // S4/S5 "project override" data. Variant 0 is the main one by
    // construction above.
    const useMain = profile.variantsPerInstruction === 1 || i % 3 !== 2
    const variant = useMain
      ? instruction.variants[0]
      : instruction.variants[instruction.variants.length - 1]
    assignments.push({
      projectId: projectIds[p],
      instructionId: instruction.id,
      variantId: variant.id,
    })
    dirtyProjects.add(p)
  }

  // --- dirty + watch passes (the fixture mirrors of DB-021 and §5.7) -------
  // Dirty (DB-021): the active half is deliberately ahead of today's
  // backend: D-096 served `dirty: false` for actives until the DB-022
  // hash pipeline landed, but fixtures model the domain rule (an
  // active project with assignments can be dirty), not the current
  // backend gap. Archived rows are NEVER dirty (DB-014/DB-021: the
  // row decides alone, pinned by a Rust test).
  //
  // Watch (§5.7, SYS-008/009): the same domain-ahead posture — the
  // backend's verdicts come from the watch service's snapshot
  // (D-137, carried by `projects_list` since T-257). The domain
  // rules: only ACTIVE, AVAILABLE rows are ever flagged (SYS-007's
  // watch set), `file_deleted` only after a recorded sync (SYS-009's
  // gate, mirrored by `last_synced_at`), and the two flags never
  // both true (the verdict's own mutual exclusion). Deterministic
  // picks over the seed-pinned synced pattern (a pure scan, NO rng
  // draws before the dirty line: its rng sequence stays
  // byte-identical): the FIRST watchable synced row is the
  // target-deleted case, the FIRST other watchable row the
  // externally-modified one — so every world with two active
  // available rows, one of them synced (set S included, pinned by
  // the spec), shows exactly one marking of each kind.
  //
  // Lost watch (SYS-010, D-155): the LAST watchable row carries
  // `unwatched_reason: 'limit'` — the non-blocking notice's fixture
  // twin (the backend serves the recorded refusal class; the watch
  // budget refusal is the representative shape). Deliberately NOT
  // exclusive with the §5.7 verdicts: a refused watch does not stop
  // the backend's verdict pass (files are read on every full pass,
  // watched or not), so the same row may be both externally modified
  // and lost from observation. The D-144 round-1 P2-2 lesson applies:
  // a marking the default world can never show is one no dev session
  // or UI test ever sees — set S's two watchable rows guarantee the
  // pick exists.
  const firstDeleted = projects.findIndex(
    (project) => project.state === 'active' && project.available && project.last_synced_at !== null,
  )
  const firstChanged = projects.findIndex(
    (project, i) => project.state === 'active' && project.available && i !== firstDeleted,
  )
  const lastWatchable = projects.reduce(
    (last, project, i) => (project.state === 'active' && project.available ? i : last),
    -1,
  )
  const finalProjects = projects.map((project, i) => {
    const watchable = project.state === 'active' && project.available
    const fileDeleted = watchable && i === firstDeleted
    const externalChanged = watchable && i === firstChanged
    return {
      ...project,
      external_changed: externalChanged,
      file_deleted: fileDeleted,
      unwatched_reason: watchable && i === lastWatchable ? 'limit' : null,
      dirty: project.state === 'active' && dirtyProjects.has(i) ? rng.bool() : false,
    }
  })

  // --- exclusions (the D-10 screen's rows, §6.2 project_exclusions) ------
  // Distinct created_at stamps keep the wire order (created_at, id)
  // meaningful: the generation order IS the oldest-first order.
  const exclusions: ExclusionFixture[] = []
  const seenExclusionIds = new Set<string>()
  const seenPathKeys = new Set<string>()
  for (let i = 0; i < profile.exclusions; i++) {
    const id = uuidv7From(rng, BASE_TS_MS + 7 * DAY_MS + i)
    if (seenExclusionIds.has(id)) {
      throw new Error(`duplicate exclusion id generated at index ${i}`)
    }
    seenExclusionIds.add(id)
    const path = `/home/dev/src/excluded/${EXCLUSION_WORDS[i % EXCLUSION_WORDS.length]}-${i}`
    const pathKey = pathKeyOf(path)
    if (seenPathKeys.has(pathKey)) {
      throw new Error(`duplicate exclusion path key generated: ${pathKey}`)
    }
    seenPathKeys.add(pathKey)
    exclusions.push({
      id,
      path,
      pathKey,
      // §6.2 CHECK vocabulary: mostly scan-removals, every third a manual
      // exclusion — both members the screen switches on.
      reason: i % 3 === 2 ? 'manual' : 'removed_scanned',
      createdAt: isoUtc(BASE_TS_MS + 7 * DAY_MS + i * DAY_MS),
    })
  }

  return { projects: finalProjects, library, assignments, exclusions }
}

/** One journal entry shape the mock `activity_list` serves (§7.7/§7.10). */
export function generateActivityEntries(
  projects: readonly ProjectFixture[],
  profile: FixtureProfile,
): ActivityEntryWire[] {
  const rng = createRng(profile.seed ^ 0x5eed)
  const entries: ActivityEntryWire[] = []
  const count = Math.min(60, Math.max(6, projects.length * 2))
  for (let i = 0; i < count; i++) {
    const action = rng.pick(ACTION_WORDS)
    const namesProject = action === 'project.added' || action === 'project.archived'
    // D-051: a settings.changed entry names its key through the entity —
    // the journal never carries the value itself.
    const namesSetting = action === 'settings.changed'
    const project = projects.length > 0 ? projects[i % projects.length] : undefined
    const level = rng.bool() ? 'info' : rng.bool() ? 'warn' : 'error'
    const result = level === 'info' ? 'ok' : rng.bool() ? 'error' : 'skipped'
    entries.push({
      id: i + 1,
      ts: isoUtc(BASE_TS_MS - (count - i) * 60 * 1000 - rng.int(0, 999)),
      level,
      // LOG-005: diagnostics never surface unless explicitly requested —
      // every 5th entry is one, so the include_diagnostic filter has data.
      visibility: i % 5 === 4 ? 'diagnostic' : 'user',
      action,
      entity_type: namesProject ? 'project' : namesSetting ? 'setting' : null,
      entity_id:
        namesProject && project ? project.id : namesSetting ? rng.pick(SETTING_KEYS) : null,
      project_id: project && rng.bool() ? project.id : null,
      sync_run_id: null,
      result,
      details: rng.bool() ? JSON.stringify({ count: rng.int(1, 9) }) : null,
    })
  }
  return entries
}

/** Copies in `db-backups/` for the recovery screen (§10, newest first). */
export function generateDbBackups(profile: FixtureProfile): DbBackupInfo[] {
  const rng = createRng(profile.seed ^ 0xbacc)
  const count = 3
  const backups: DbBackupInfo[] = []
  for (let i = 0; i < count; i++) {
    backups.push({
      name: `daily-${String(i + 1).padStart(3, '0')}.sqlite3`,
      size_bytes: rng.int(2048, 1_048_576),
      modified_utc: isoUtc(BASE_TS_MS - (count - i) * DAY_MS + rng.int(0, 3600) * 1000),
    })
  }
  // Newest first, the order the screen renders.
  return backups.reverse()
}
