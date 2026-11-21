import { describe, expect, it } from 'vitest'
import { defaultLabel, pathKeyOf, type InstructionFixture } from './domain'
import {
  ACTION_MEMBERS,
  FIXTURE_PROFILE_L,
  FIXTURE_PROFILE_M,
  FIXTURE_PROFILE_S,
  generateActivityEntries,
  generateFixtures,
  parseFixtureProfile,
  PALETTE,
  type FixtureProfile,
} from './generator'
import { SETTING_KEYS } from './settings'
import { createRng, uuidv7From } from './rng'

/** A tiny custom world with all vocabulary cases in it. */
const PROFILE: FixtureProfile = {
  seed: 7,
  projects: 7,
  instructions: 4,
  headings: 2,
  assignments: 10,
  archivedProjects: 2,
  variantsPerInstruction: 2,
  exclusions: 3,
}

describe('generateFixtures: determinism', () => {
  it('reproduces the same bundle for the same profile', () => {
    expect(generateFixtures(PROFILE)).toEqual(generateFixtures(PROFILE))
  })

  it('changes the data when the seed changes', () => {
    const other = generateFixtures({ ...PROFILE, seed: PROFILE.seed + 1 })
    const base = generateFixtures(PROFILE)
    const sameNames = base.projects.every((p, i) => p.name === other.projects[i]?.name)
    expect(sameNames).toBe(false)
  })

  it('produces equal output for the shipped set S twice (deep)', () => {
    expect(generateFixtures(FIXTURE_PROFILE_S)).toEqual(generateFixtures(FIXTURE_PROFILE_S))
  })
})

describe('generateFixtures: profile shape', () => {
  const bundle = generateFixtures(PROFILE)

  it('honors every count', () => {
    expect(bundle.projects).toHaveLength(PROFILE.projects)
    expect(bundle.assignments).toHaveLength(PROFILE.assignments)
    const instructions = bundle.library.filter((i) => i.kind === 'instruction')
    const headings = bundle.library.filter((i) => i.kind === 'heading')
    expect(instructions).toHaveLength(PROFILE.instructions)
    expect(headings).toHaveLength(PROFILE.headings)
  })

  it('archives exactly the last N projects and keeps them in the list', () => {
    const states = bundle.projects.map((p) => p.state)
    expect(states).toEqual([
      ...Array.from({ length: PROFILE.projects - PROFILE.archivedProjects }, () => 'active'),
      ...Array.from({ length: PROFILE.archivedProjects }, () => 'archived'),
    ])
  })

  it('builds library positions as a gapless 1..N sequence over both kinds', () => {
    const positions = bundle.library.map((i) => i.position).sort((a, b) => a - b)
    expect(positions).toEqual(bundle.library.map((_, i) => i + 1))
  })

  it('marks every 11th project unavailable with a D-084 reason', () => {
    // 7 projects → no unavailable row here; a longer profile exercises it.
    const longer = generateFixtures({ ...PROFILE, projects: 23, assignments: 23 })
    const reasons = longer.projects.filter((p) => !p.available).map((p) => p.unavailable_reason)
    // Indices 10 and 21 (every 11th); reason words cycle the D-084 list
    // (10 % 4 = 2 → not_readable, 21 % 4 = 1 → not_a_directory).
    expect(reasons).toEqual(['not_readable', 'not_a_directory'])
    // The available majority carries no reason.
    const available = longer.projects.filter((p) => p.available)
    expect(available).toHaveLength(21)
    expect(available.every((p) => p.unavailable_reason === null)).toBe(true)
  })
})

describe('defaultLabel (DB-042)', () => {
  // Literals from the rule itself (the Rust pin's examples), not from the
  // implementation: upper-case FIRST, then the first up to 6 code points
  // of the trimmed name — `straße` must fold to STRASS, never the
  // 7-code-point STRASSE a slice-first derivation would produce.
  it.each([
    ['My Project', 'MY PRO'],
    ['straße', 'STRASS'],
    ['  EunomiaSync  ', 'EUNOMI'],
    ['ab', 'AB'],
    ['ÄÖÜäöü', 'ÄÖÜÄÖÜ'],
    ['𝕬𝕭𝕮𝕯𝕰𝕱', '𝕬𝕭𝕮𝕯𝕰𝕱'],
  ])('%p → %p', (name, label) => {
    expect(defaultLabel(name)).toBe(label)
  })

  it('never returns an empty label for a non-blank name', () => {
    expect(defaultLabel('x')).toBe('X')
    expect(defaultLabel('   x   ')).toBe('X')
  })
})

describe('generateFixtures: domain rules', () => {
  const bundle = generateFixtures(PROFILE)

  it('derives every label with the DB-042 rule (length 1..6)', () => {
    for (const project of bundle.projects) {
      expect(project.label).toBe(defaultLabel(project.name))
      expect(project.label.length).toBeLessThanOrEqual(6)
      expect(project.label.length).toBeGreaterThan(0)
    }
  })

  it('uses only DB-040 palette colors', () => {
    for (const project of bundle.projects) {
      expect(PALETTE).toContain(project.color)
    }
  })

  it('keeps every id unique across projects, items and variants', () => {
    const ids = [
      ...bundle.projects.map((p) => p.id),
      ...bundle.library.map((i) => i.id),
      ...bundle.library.flatMap((i) =>
        i.kind === 'instruction' ? i.variants.map((v) => v.id) : [],
      ),
    ]
    expect(new Set(ids).size).toBe(ids.length)
  })

  it('gives every instruction exactly one main variant named Main', () => {
    for (const item of bundle.library) {
      if (item.kind !== 'instruction') continue
      const main = item.variants.filter((v) => v.isMain)
      expect(main).toHaveLength(1)
      expect(main[0]?.name).toBe('Main')
    }
  })

  it('references only real rows: every assignment is a sound (project, instruction, variant) triple', () => {
    const projectIds = new Set(bundle.projects.map((p) => p.id))
    const instructionsById = new Map(
      bundle.library
        .filter((i): i is InstructionFixture => i.kind === 'instruction')
        .map((i) => [i.id, i] as const),
    )
    const pairs = new Set<string>()
    for (const assignment of bundle.assignments) {
      expect(projectIds.has(assignment.projectId)).toBe(true)
      const instruction = instructionsById.get(assignment.instructionId)
      expect(instruction, 'assignment names a real instruction').toBeDefined()
      const variantIds = instruction ? instruction.variants.map((v) => v.id) : []
      expect(variantIds).toContain(assignment.variantId)
      const pair = `${assignment.projectId}:${assignment.instructionId}`
      expect(pairs.has(pair), `duplicate pair ${pair}`).toBe(false)
      pairs.add(pair)
    }
  })

  it('never assigns a hidden instruction (profile with real hidden rows)', () => {
    // 15 instructions cross the i % 13 === 12 gate, so the bundle HAS a
    // hidden instruction — the assertion is only meaningful with one.
    const hiddenProfile = generateFixtures({
      ...PROFILE,
      projects: 3,
      instructions: 15,
      headings: 0,
      assignments: 20,
      archivedProjects: 0,
    })
    const hiddenIds = new Set(
      hiddenProfile.library
        .filter((i) => i.kind === 'instruction' && i.state === 'hidden')
        .map((i) => i.id),
    )
    expect(hiddenIds.size).toBeGreaterThan(0)
    expect(
      hiddenProfile.assignments.some((a) => hiddenIds.has(a.instructionId)),
      'a hidden instruction must not receive assignments',
    ).toBe(false)
  })

  it('keeps the default dev world covering the hidden vocabulary (set S)', () => {
    const s = generateFixtures(FIXTURE_PROFILE_S)
    expect(
      s.library.some((i) => i.kind === 'instruction' && i.state === 'hidden'),
      'set S should include one hidden instruction for the library UI',
    ).toBe(true)
  })
})

describe('generateActivityEntries: closed vocabularies', () => {
  it('ACTION_MEMBERS mirrors the store vocabulary exactly once over (D-108/D-119)', () => {
    // The D-108 strict-coverage mirror: the mock's `activity_list`
    // filter validates against this list, so a member lost here
    // would make the mock reject a filter value the real boundary
    // accepts. The length literal (22, `Action::ALL` today) catches
    // an accidental deletion; the dedup check catches a copy-paste
    // double. No cross-language oracle exists (bindings.ts carries no
    // enum), so the two literals are the pin.
    expect(ACTION_MEMBERS).toHaveLength(22)
    expect(new Set(ACTION_MEMBERS).size).toBe(ACTION_MEMBERS.length)
  })

  it('writes only real action_log members and their enums', () => {
    const bundle = generateFixtures(PROFILE)
    const entries = generateActivityEntries(bundle.projects, PROFILE)
    expect(entries.length).toBeGreaterThan(0)
    for (const entry of entries) {
      expect(ACTION_MEMBERS, `action ${entry.action}`).toContain(entry.action)
      expect(['info', 'warn', 'error']).toContain(entry.level)
      expect(['user', 'diagnostic']).toContain(entry.visibility)
      expect(['ok', 'error', 'skipped']).toContain(entry.result)
      if (entry.entity_type !== null) {
        expect(['project', 'setting']).toContain(entry.entity_type)
      }
    }
    // The §6.2 wire shape: DB-002 timestamps, some details present.
    expect(entries.every((e) => /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/.test(e.ts))).toBe(
      true,
    )
    expect(entries.some((e) => e.details !== null)).toBe(true)
  })

  it('journals a settings.changed entry under its key entity (D-051)', () => {
    const bundle = generateFixtures(PROFILE)
    const entries = generateActivityEntries(bundle.projects, PROFILE)
    const settingsEntries = entries.filter((e) => e.action === 'settings.changed')
    expect(settingsEntries.length).toBeGreaterThan(0)
    for (const entry of settingsEntries) {
      expect(entry.entity_type).toBe('setting')
      expect(SETTING_KEYS).toContain(entry.entity_id)
    }
  })
})

describe('generateFixtures: dirty rules (DB-021)', () => {
  const bundle = generateFixtures(PROFILE)

  it('flags dirty only on projects that carry assignments', () => {
    const assigned = new Set(bundle.assignments.map((a) => a.projectId))
    for (const project of bundle.projects) {
      if (!assigned.has(project.id)) {
        expect(project.dirty, `${project.name} has no assignments, cannot be dirty`).toBe(false)
      }
    }
    const dirty = bundle.projects.filter((p) => p.dirty).map((p) => p.id)
    expect(dirty.every((id) => assigned.has(id))).toBe(true)
  })

  it('never flags an archived project dirty (DB-014/DB-021)', () => {
    for (const project of generateFixtures(FIXTURE_PROFILE_S).projects) {
      if (project.state === 'archived') {
        expect(project.dirty, `${project.name} is archived, never dirty`).toBe(false)
      }
    }
    const several = generateFixtures({
      ...PROFILE,
      projects: 10,
      assignments: 20,
      archivedProjects: 4,
    })
    expect(several.projects.filter((p) => p.state === 'archived')).toHaveLength(4)
    expect(
      several.projects.filter((p) => p.state === 'archived' && p.dirty),
      'archived rows are never dirty',
    ).toEqual([])
  })
})

describe('generateFixtures: rejection of impossible profiles', () => {
  it('refuses more assignments than the unique pair space', () => {
    expect(() => generateFixtures({ ...PROFILE, projects: 2, assignments: 99 })).toThrow(
      /pair space/,
    )
  })

  it('refuses more archived projects than projects', () => {
    expect(() => generateFixtures({ ...PROFILE, archivedProjects: 99 })).toThrow(/archived/)
  })

  it('refuses zero variants per instruction', () => {
    expect(() => generateFixtures({ ...PROFILE, variantsPerInstruction: 0 })).toThrow(/variant/)
  })
})

describe('generateFixtures: exclusions (the D-10 rows)', () => {
  it('generates the requested rows with unique ids and path keys', () => {
    const bundle = generateFixtures(PROFILE)
    expect(bundle.exclusions).toHaveLength(PROFILE.exclusions)
    const ids = bundle.exclusions.map((e) => e.id)
    const keys = bundle.exclusions.map((e) => e.pathKey)
    expect(new Set(ids).size).toBe(ids.length)
    expect(new Set(keys).size).toBe(keys.length)
  })

  it('keeps the §6.2 reason vocabulary and the DB-030 key transform', () => {
    const bundle = generateFixtures(PROFILE)
    for (const row of bundle.exclusions) {
      expect(['removed_scanned', 'manual']).toContain(row.reason)
      expect(row.pathKey).toBe(pathKeyOf(row.path))
      expect(row.createdAt).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/)
    }
    // PROFILE's third row is the manual member; set S (two rows) is all
    // scan-removals — both spellings the screen switches on are covered.
    expect(new Set(bundle.exclusions.map((e) => e.reason))).toEqual(
      new Set(['removed_scanned', 'manual']),
    )
    const s = generateFixtures(FIXTURE_PROFILE_S)
    expect(s.exclusions).toHaveLength(2)
    expect(s.exclusions.every((e) => e.reason === 'removed_scanned')).toBe(true)
  })
})

describe('presets (spec §12.2)', () => {
  it('set M: 50 projects, 200 instructions', () => {
    expect(FIXTURE_PROFILE_M.projects).toBe(50)
    expect(FIXTURE_PROFILE_M.instructions).toBe(200)
  })

  it('set L: the upper-limit numbers, as parameters only', () => {
    // Asserted as profile values, not generated data: the unit suite never
    // builds set L (README documents how to opt in for scroll work).
    expect(FIXTURE_PROFILE_L).toEqual({
      seed: expect.any(Number),
      projects: 500,
      instructions: 1000,
      headings: 100,
      assignments: 100000,
      archivedProjects: expect.any(Number),
      variantsPerInstruction: 3,
      exclusions: expect.any(Number),
    })
  })

  it('parseFixtureProfile maps the dev knob onto the presets', () => {
    expect(parseFixtureProfile(undefined)).toBe(FIXTURE_PROFILE_S)
    expect(parseFixtureProfile('s')).toBe(FIXTURE_PROFILE_S)
    expect(parseFixtureProfile('M')).toBe(FIXTURE_PROFILE_M)
    expect(parseFixtureProfile('l')).toBe(FIXTURE_PROFILE_L)
    expect(parseFixtureProfile('garbage')).toBe(FIXTURE_PROFILE_S)
  })
})

describe('rng', () => {
  it('uuidv7From builds the 8-4-4-4-12 shape with version 7 and RFC variant', () => {
    const rng = createRng(1)
    const id = uuidv7From(rng, Date.UTC(2026, 10, 20))
    expect(id).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/)
  })

  it('createRng repeats its sequence for the same seed', () => {
    const a = createRng(42)
    const b = createRng(42)
    const draw = (r: ReturnType<typeof createRng>) => [r.next(), r.next(), r.int(1, 100)]
    expect(draw(a)).toEqual(draw(b))
  })
})
