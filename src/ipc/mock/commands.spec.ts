import { beforeEach, describe, expect, it } from 'vitest'
import { mockCommands } from './commands'
import { mockState, resetMockState, type MockLibraryRow, type MockProjectRow } from './state'
import { mockEvents } from './events'
import { BASE_TS_MS, PALETTE, type FixtureProfile } from '../fixtures/generator'
import { isoUtc } from '../fixtures/rng'
import { pathKeyOf } from '../fixtures/domain'
import type { ActivityEntryWire, AppErrorWire, LibraryTreeItemWire } from '../contract'

/** A small pinned world: 4 projects (last one archived), 3 instructions. */
const PROFILE: FixtureProfile = {
  seed: 21,
  projects: 4,
  instructions: 3,
  headings: 1,
  assignments: 5,
  archivedProjects: 1,
  variantsPerInstruction: 1,
  exclusions: 2,
}

const commands = mockCommands

beforeEach(() => {
  resetMockState(PROFILE)
})

describe('projectsList', () => {
  it('serves active projects only by default, in column order', async () => {
    const projects = await commands.projectsList(null)
    const rows = mockState().projects
    expect(projects).toHaveLength(rows.filter((r) => r.state === 'active').length)
    // `sort_order` is not a wire field; the wire order itself is the
    // contract (ALG-001), so the names must come back in row order.
    const expected = [...rows]
      .filter((r) => r.state === 'active')
      .sort((a, b) => a.sortOrder - b.sortOrder)
      .map((r) => r.name)
    expect(projects.map((p) => p.name)).toEqual(expected)
  })

  it('includes archived entries behind the flag, keeping their positions', async () => {
    // Re-scatter: SWAP the archived row into a middle position (§6.4 keeps
    // an archived row's place), then the widened read must serve the same
    // order with it in place.
    const rows = mockState().projects
    const archived = rows.find((r) => r.state === 'archived')
    expect(archived).toBeDefined()
    const displaced = rows.find((r) => r.sortOrder === 2)
    if (archived && displaced && archived !== displaced) {
      const swap = archived.sortOrder
      archived.sortOrder = displaced.sortOrder
      displaced.sortOrder = swap
    }
    const all = await commands.projectsList(true)
    const idsByOrder = all.map((p) => p.id)
    const expected = [...rows].sort((a, b) => a.sortOrder - b.sortOrder).map((r) => r.id)
    expect(idsByOrder).toEqual(expected)
    expect(idsByOrder.indexOf(archived?.id ?? '')).toBe(1)
  })

  it('carries the full T-072 column shape', async () => {
    const [first] = await commands.projectsList(null)
    expect(first).toBeDefined()
    expect(Object.keys(first ?? {}).sort()).toEqual([
      'available',
      'color',
      'dirty',
      'external_changed',
      'file_deleted',
      'id',
      'label',
      'last_synced_at',
      'name',
      'state',
      'unavailable_reason',
      'unwatched_reason',
    ])
  })

  it('serves the §5.7 watch flags from the rows, never both at once', async () => {
    // T-257's carrier, mock side: the flags are row state mapped onto
    // the wire (a UI test edits the row, exactly like the write-access
    // warning arms). The pinned PROFILE world also pins the
    // generator's deterministic picks (seed 21): index 0 — the first
    // synced active row — is the target-deleted case, index 1 the
    // externally-modified one.
    const rows = mockState().projects
    const clean = rows[0]!
    clean.externalChanged = false
    clean.fileDeleted = false
    const modified = rows[1]!
    modified.externalChanged = true
    modified.fileDeleted = false
    const deleted = rows[2]!
    deleted.externalChanged = false
    deleted.fileDeleted = true

    const served = await commands.projectsList(null)
    const byId = new Map(served.map((p) => [p.id, p]))
    expect(byId.get(clean.id)?.external_changed).toBe(false)
    expect(byId.get(clean.id)?.file_deleted).toBe(false)
    expect(byId.get(modified.id)?.external_changed).toBe(true)
    expect(byId.get(modified.id)?.file_deleted).toBe(false)
    expect(byId.get(deleted.id)?.file_deleted).toBe(true)
    expect(byId.get(deleted.id)?.external_changed).toBe(false)

    // The SYS-010 lost-watch notice (D-155) rides the same carrier:
    // row state mapped onto the wire, in the closed class vocabulary
    // — a UI test edits the row, exactly like the flag arms above.
    clean.unwatchedReason = 'limit'
    modified.unwatchedReason = 'gone'
    const noticed = await commands.projectsList(null)
    const noticedById = new Map(noticed.map((p) => [p.id, p]))
    expect(noticedById.get(clean.id)?.unwatched_reason).toBe('limit')
    expect(noticedById.get(modified.id)?.unwatched_reason).toBe('gone')
    expect(noticedById.get(deleted.id)?.unwatched_reason, 'the untouched row keeps its value').toBe(
      deleted.unwatchedReason,
    )

    // The generator invariants over the fresh pinned world (reset:
    // the edits above must not leak into them). The deterministic
    // picks over seed 21's synced pattern: index 0 (the first synced
    // active row) is the deleted case, index 1 the modified one.
    resetMockState(PROFILE)
    const generated = await commands.projectsList(null)
    expect(
      generated.every((p) => !(p.external_changed && p.file_deleted)),
      'SYS-008/009 verdicts are mutually exclusive',
    ).toBe(true)
    expect(generated.filter((p) => p.external_changed)).toHaveLength(1)
    expect(generated.filter((p) => p.file_deleted)).toHaveLength(1)
    expect(
      generated.every((p) => !p.file_deleted || p.last_synced_at !== null),
      'SYS-009: deleted only after a recorded sync',
    ).toBe(true)
    expect(
      generated.filter((p) => p.unwatched_reason !== null),
      'SYS-010: exactly one lost watch, the last watchable row',
    ).toHaveLength(1)
    expect(
      generated.every((p) => p.unwatched_reason === null || p.unwatched_reason === 'limit'),
      'the notice serves the closed class vocabulary',
    ).toBe(true)
    const archived = (await commands.projectsList(true)).find((p) => p.state === 'archived')
    expect(archived?.external_changed).toBe(false)
    expect(archived?.file_deleted).toBe(false)
    expect(archived?.unwatched_reason, 'an archived row is outside the watch set').toBeNull()
  })
})

describe('libraryGet (the §7.3 tree, T-083/D-187)', () => {
  /** Narrows the wire union to its instruction arm. */
  const isInstruction = (
    entry: LibraryTreeItemWire,
  ): entry is Extract<LibraryTreeItemWire, { kind: 'instruction' }> => entry.kind === 'instruction'

  /** Narrows the wire union to its heading arm. */
  const isHeading = (
    entry: LibraryTreeItemWire,
  ): entry is Extract<LibraryTreeItemWire, { kind: 'heading' }> => entry.kind === 'heading'

  /** One hand-built heading row (positions and ids chosen by the test). */
  function headingRow(id: string, position: number, level: 2 | 3 | 4 | 5 | 6, title: string) {
    return {
      kind: 'heading' as const,
      id,
      position,
      headingLevel: level,
      title,
      state: 'active' as const,
      createdAt: '2026-11-20T09:00:00.000Z',
      updatedAt: '2026-11-20T09:00:00.000Z',
    }
  }

  /** One hand-built instruction row with its variants. */
  function instructionRow(
    id: string,
    position: number,
    title: string,
    state: 'active' | 'hidden',
    variants: { id: string; name: string; content: string; isMain: boolean }[],
  ) {
    return {
      kind: 'instruction' as const,
      id,
      position,
      title,
      state,
      // The variant-command columns (T-329's union extension) filled
      // with the hand-built world's fixed date: a fixture-shaped
      // literal list becomes a complete `MockVariantRow` list.
      variants: variants.map((variant) => ({
        ...variant,
        state: 'active' as const,
        createdAt: '2026-11-20T09:00:01.000Z',
        updatedAt: '2026-11-20T09:00:01.000Z',
        deletedAt: null,
      })),
      createdAt: '2026-11-20T09:00:01.000Z',
      updatedAt: '2026-11-20T09:00:01.000Z',
    }
  }

  it('serves the tree in position order with both kinds tagged and the full field set', async () => {
    const rows = mockState().library
    const answer = await commands.libraryGet(null)
    // D-200: the answer's own key set is pinned, and the D-14 number
    // is zero here — the generator's PROFILE holds no hidden row.
    expect(Object.keys(answer).sort()).toEqual(['hidden_count', 'tree'])
    expect(answer.hidden_count).toBe(0)
    const tree = answer.tree
    // The served order is the rows' position order; the default read
    // serves the visible library (the generator hides every 13th
    // instruction — with PROFILE's 4 items, none is hidden, so the
    // default read serves every row here).
    expect(tree.map((entry) => entry.id)).toEqual(
      [...rows].sort((a, b) => a.position - b.position).map((row) => row.id),
    )
    // Both kinds are present and tagged (§6.2's `kind` literals).
    expect(tree.some(isHeading)).toBe(true)
    expect(tree.some(isInstruction)).toBe(true)
    // The instruction half carries the full wire field set, the DB-002
    // date shape, and the store's variant order.
    const first = tree.find(isInstruction)
    expect(first).toBeDefined()
    expect(Object.keys(first ?? {}).sort()).toEqual([
      'created_at',
      'id',
      'kind',
      'project_labels',
      'state',
      'title',
      'updated_at',
      'variant_count',
      'variants',
    ])
    expect(first?.created_at).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/)
    expect(first?.updated_at).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/)
    const row = rows.find((r) => r.kind === 'instruction' && r.id === first?.id)
    expect(first?.variant_count).toBe(row?.kind === 'instruction' ? row.variants.length : 0)
    expect(first?.variants.map((v) => [v.id, v.name])).toEqual(
      row?.kind === 'instruction' ? row.variants.map((v) => [v.id, v.name]) : [],
    )
    // The heading half carries its own field set.
    const heading = tree.find(isHeading)
    expect(Object.keys(heading ?? {}).sort()).toEqual([
      'from_level',
      'id',
      'kind',
      'level',
      'level_skipped',
      'title',
    ])
    expect(heading?.level).toBeGreaterThanOrEqual(2)
    expect(heading?.level).toBeLessThanOrEqual(6)
  })

  it('labels are exactly the active projects with an assignment, naming the assigned variant', async () => {
    // The independent oracle: the test joins the store's own rows
    // (projects in sort_order × their assignment rows) — the wire must
    // agree with that join, per instruction.
    const state = mockState()
    const expected = new Map<string, { project_id: string; variant_id: string | null }[]>()
    for (const project of [...state.projects]
      .sort((a, b) => a.sortOrder - b.sortOrder)
      .filter((p) => p.state === 'active')) {
      for (const assignment of state.assignments.filter((a) => a.projectId === project.id)) {
        const list = expected.get(assignment.instructionId) ?? []
        list.push({
          project_id: project.id,
          // MAIN-034: a variant target names its variant, an extend
          // cell its base; only a replace cell names none.
          variant_id:
            assignment.target.kind === 'variant'
              ? assignment.target.value
              : assignment.target.kind === 'extend'
                ? assignment.target.value.base
                : null,
        })
        expected.set(assignment.instructionId, list)
      }
    }
    const tree = (await commands.libraryGet(null)).tree
    const labeled = tree.filter(isInstruction)
    expect(labeled.length).toBeGreaterThan(0)
    for (const instruction of labeled) {
      expect(
        instruction.project_labels.map((l) => [l.project_id, l.variant_id]),
        `labels of ${instruction.title}`,
      ).toEqual((expected.get(instruction.id) ?? []).map((l) => [l.project_id, l.variant_id]))
    }
    // The chip's own fields come from the project row.
    const withLabel = labeled.find((instruction) => instruction.project_labels.length > 0)
    expect(withLabel).toBeDefined()
    const label = withLabel?.project_labels[0]
    const project = mockState().projects.find((p) => p.id === label?.project_id)
    expect(label?.name).toBe(project?.name)
    expect(label?.label).toBe(project?.label)
    expect(label?.color).toBe(project?.color)

    // The archive flip (DB-014): archiving a labeled project removes
    // its chip everywhere while the assignment rows stay in the store.
    const victim = mockState().projects.find(
      (p) => p.state === 'active' && state.assignments.some((a) => a.projectId === p.id),
    )
    expect(victim).toBeDefined()
    if (victim) {
      victim.state = 'archived'
      const relisted = (await commands.libraryGet(null)).tree
      const stillLabeled = relisted
        .filter(isInstruction)
        .some((instruction) => instruction.project_labels.some((l) => l.project_id === victim.id))
      expect(stillLabeled, 'an archived project never labels').toBe(false)
      expect(
        mockState().assignments.some((a) => a.projectId === victim.id),
        'the archived project keeps its assignment rows (DB-014)',
      ).toBe(true)
    }
  })

  it('marks an empty variant with the LIB-014 flag and the main flag', async () => {
    const rows = mockState().library
    const target = rows.find((r) => r.kind === 'instruction')
    expect(target).toBeDefined()
    if (target?.kind === 'instruction') {
      target.variants[0]!.content = ''
      const tree = (await commands.libraryGet(null)).tree
      const served = tree.filter(isInstruction).find((instruction) => instruction.id === target.id)
      expect(served?.variants[0]?.is_empty, 'the empty variant is marked (LIB-014)').toBe(true)
      expect(served?.variants[0]?.is_main, 'the main flag rides the same list (DB-010)').toBe(true)
      // D-13's reading: whitespace-only text outputs no text, so it is
      // as empty as "" — the mock mirrors the backend's ALG-002-based
      // flag. The BOM rule is positional (round-2 P3-1): a trailing
      // BOM blocks the right-hand strip, a leading one behind blank
      // lines does not.
      target.variants[0]!.content = ' \n\t '
      const whitespace = (await commands.libraryGet(null)).tree
      const servedBlank = whitespace
        .filter(isInstruction)
        .find((instruction) => instruction.id === target.id)
      expect(servedBlank?.variants[0]?.is_empty, 'whitespace-only outputs no text (D-13)').toBe(
        true,
      )
      target.variants[0]!.content = ' \u{FEFF}'
      const trailingBom = (await commands.libraryGet(null)).tree
      const servedTrailing = trailingBom
        .filter(isInstruction)
        .find((instruction) => instruction.id === target.id)
      expect(
        servedTrailing?.variants[0]?.is_empty,
        'a TRAILING BOM blocks the strip: the variant outputs text',
      ).toBe(false)
      target.variants[0]!.content = '\n\u{FEFF}'
      const leadingBom = (await commands.libraryGet(null)).tree
      const servedLeading = leadingBom
        .filter(isInstruction)
        .find((instruction) => instruction.id === target.id)
      expect(
        servedLeading?.variants[0]?.is_empty,
        'a BOM directly behind a dropped blank line is leading: nothing outputs',
      ).toBe(true)
      // The flag is false for a filled variant.
      const filled = leadingBom
        .filter(isInstruction)
        .find((instruction) => instruction.variants.some((v) => !v.is_empty))
      expect(filled?.variants.some((v) => !v.is_empty)).toBe(true)
    }
  })

  it('serves the tree in position order even when the store array is not', async () => {
    // The P3-7 pin (round-2 P3-2): the wire contract's order is the
    // row's `position`, never the array order the store happens to
    // hold — reverse the array, keep the positions, and the served
    // ids must still climb.
    const rows = mockState().library
    const byPosition = [...rows].sort((a, b) => a.position - b.position).map((row) => row.id)
    mockState().library.reverse()
    const tree = (await commands.libraryGet(null)).tree
    expect(tree.map((entry) => entry.id)).toEqual(byPosition)
  })

  it('flags only the headings that arrive too deep (LIB-032)', async () => {
    // The spec's own example set: `##` then `####` is a skip; one step
    // deeper, a close and a sibling are not; the first heading has no
    // pair. Hand-built world, so the expected flags are literals.
    mockState().library = [
      headingRow('h-style', 1, 2, 'Style'),
      instructionRow('i-shy', 2, 'Shy', 'hidden', [
        { id: 'v-shy', name: 'Main', content: 'Be brief.', isMain: true },
      ]),
      headingRow('h-tone', 3, 4, 'Tone'),
      headingRow('h-deep', 4, 5, 'Deep'),
      headingRow('h-release', 5, 3, 'Release'),
      headingRow('h-max', 6, 6, 'Max'),
      headingRow('h-back', 7, 3, 'Back'),
    ]
    const flags = async (flag: boolean | null) => {
      const tree = (await commands.libraryGet(flag)).tree
      return tree
        .filter(isHeading)
        .map((heading) => [heading.title, heading.level_skipped, heading.from_level] as const)
    }
    // The flag set is the same under either scope (hidden instructions
    // are transparent to the pair walk) — and D-199's second fact
    // rides exactly the skip rows: the level each skip arrives FROM
    // (two DIFFERENT values — H2 and H3 — so a constant cannot pass).
    const expected = [
      ['Style', false, null],
      ['Tone', true, 2],
      ['Deep', false, null],
      ['Release', false, null],
      ['Max', true, 3],
      ['Back', false, null],
    ] as const
    expect(await flags(null)).toEqual(expected)
    expect(await flags(true)).toEqual(expected)
  })

  it('hidden instructions appear behind the flag with their labels kept (DB-013)', async () => {
    const state = mockState()
    const active = state.projects.find((p) => p.state === 'active')
    expect(active).toBeDefined()
    mockState().library = [
      instructionRow('i-keeper', 1, 'Keeper', 'hidden', [
        { id: 'v-keeper', name: 'Main', content: 'Keep.', isMain: true },
      ]),
      instructionRow('i-dropper', 2, 'Dropper', 'active', [
        { id: 'v-dropper', name: 'Main', content: 'Drop.', isMain: true },
      ]),
    ]
    mockState().assignments = [
      {
        projectId: active!.id,
        instructionId: 'i-keeper',
        target: { kind: 'variant', value: 'v-keeper' },
        createdAt: '2026-11-20T00:00:00.000Z',
        updatedAt: '2026-11-20T00:00:00.000Z',
      },
    ]
    const visible = await commands.libraryGet(null)
    expect(visible.tree.map((entry) => entry.title)).toEqual(['Dropper'])
    const widened = await commands.libraryGet(true)
    const keeper = widened.tree
      .filter(isInstruction)
      .find((instruction) => instruction.title === 'Keeper')
    expect(keeper?.state).toBe('hidden')
    expect(keeper?.project_labels.map((l) => l.project_id)).toEqual([active!.id])
    // D-200: the count is ONE hidden instruction, the same number
    // under both scopes (the default answer knows its hidden half
    // without serving it).
    expect(visible.hidden_count).toBe(1)
    expect(widened.hidden_count).toBe(1)
  })
})

describe('projectsReorder', () => {
  it('splices the new active order around the archived row and journals the moved count', async () => {
    // D-093/D-115's composition: the archived row keeps its own SLOT,
    // the active slots take the reversed active order, the table
    // renumbers gaplessly, and one project.changed entry carries the
    // moved count.
    const rows = mockState().projects
    const archived = rows.find((r) => r.state === 'archived')
    expect(archived).toBeDefined()
    // Scatter the archived row into a middle slot first (§6.4 keeps
    // its place), like the projectsList pin does.
    const displaced = rows.find((r) => r.state === 'active' && r.sortOrder === 2)
    if (archived && displaced && archived !== displaced) {
      const swap = archived.sortOrder
      archived.sortOrder = displaced.sortOrder
      displaced.sortOrder = swap
    }
    const sorted = [...rows].sort((a, b) => a.sortOrder - b.sortOrder)
    const actives = sorted.filter((r) => r.state === 'active')
    expect(actives.length).toBeGreaterThanOrEqual(2)
    const reversed = [...actives].reverse()

    await expect(commands.projectsReorder(reversed.map((r) => r.id))).resolves.toBeNull()

    const after = [...mockState().projects].sort((a, b) => a.sortOrder - b.sortOrder)
    expect(after.filter((r) => r.state === 'active').map((r) => r.id)).toEqual(
      reversed.map((r) => r.id),
    )
    // The archived row keeps its own slot (D-093/D-115).
    expect(after[1]?.id).toBe(archived?.id)
    expect(after.map((r) => r.sortOrder)).toEqual(after.map((_, index) => index + 1))
    const page = await commands.activityList({ actions: ['project.changed'], limit: 500 })
    const entry = page.entries.find((e) => e.details?.startsWith('{"changed":'))
    expect(entry, 'one project.changed entry carrying the moved count').toBeDefined()
    expect(entry?.visibility).toBe('user')
    expect(entry?.result).toBe('ok')
    expect(entry?.entity_type).toBeNull()
  })

  it('answers a re-sent unchanged order as a no-op, journaling nothing', async () => {
    const sorted = [...mockState().projects]
      .sort((a, b) => a.sortOrder - b.sortOrder)
      .filter((r) => r.state === 'active')
    const journalLength = mockState().activity.length

    await expect(commands.projectsReorder(sorted.map((r) => r.id))).resolves.toBeNull()

    expect(mockState().activity).toHaveLength(journalLength)
  })

  it('rejects an archived id alongside a complete active set with the typed envelope', async () => {
    const sorted = [...mockState().projects]
      .sort((a, b) => a.sortOrder - b.sortOrder)
      .filter((r) => r.state === 'active')
    const archivedId = mockState().projects.find((r) => r.state === 'archived')?.id ?? ''
    const journalLength = mockState().activity.length

    await expect(
      commands.projectsReorder([...sorted.map((r) => r.id), archivedId]),
    ).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'ordered_ids',
        value: archivedId,
        reason:
          'the list must name every active project exactly once: 1 not an active project, 0 missing, 0 repeated',
      },
    })

    expect(mockState().activity).toHaveLength(journalLength)
  })

  it('rejects the empty list and a repeated id, changing nothing', async () => {
    const journalLength = mockState().activity.length

    await expect(commands.projectsReorder([])).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'ordered_ids', value: '0 ids sent' },
    })

    const sorted = [...mockState().projects]
      .sort((a, b) => a.sortOrder - b.sortOrder)
      .filter((r) => r.state === 'active')
    const [first] = sorted
    await expect(
      commands.projectsReorder([...sorted.map((r) => r.id), first?.id ?? '']),
    ).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { reason: expect.stringContaining('1 repeated') },
    })

    expect(mockState().activity).toHaveLength(journalLength)
  })
})

describe('projectAddManual', () => {
  it('adds a picked folder with derived fields and serves it back', async () => {
    const before = await commands.projectsList(true)
    const maxOrder = mockState().projects.reduce((m, p) => Math.max(m, p.sortOrder), 0)
    const result = await commands.projectAddManual('/home/dev/src/new-repo', null)
    expect(result.outcome).toBe('added')
    expect(result.project?.name).toBe('new-repo')
    expect(result.project?.label).toBe('NEW-RE')
    expect(result.project?.added_via).toBe('manual')
    expect(result.project?.scheme).toBe('A')
    expect(result.project?.base_name).toBe('AGENTS.md')
    expect(result.project?.write_access_warning).toBeNull()
    expect(result.project?.sort_order).toBe(maxOrder + 1)
    const after = await commands.projectsList(true)
    expect(after.length).toBe(before.length + 1)
    expect(after.some((p) => p.name === 'new-repo')).toBe(true)
  })

  it('copies the CURRENT default.* settings into the new row (PRJ-012)', async () => {
    await commands.settingsSet([
      { key: 'default.scheme', value: '"C"' },
      { key: 'default.base_name', value: '"CLAUDE.md"' },
    ])
    const result = await commands.projectAddManual('/home/dev/src/defaults-repo', null)
    expect(result.project?.scheme).toBe('C')
    expect(result.project?.base_name).toBe('CLAUDE.md')
    // Later default changes never reach it (DEF-005): the row keeps its own.
    await commands.settingsSet([{ key: 'default.scheme', value: '"B"' }])
    const reread = await commands.projectsList(true)
    expect(reread.find((p) => p.name === 'defaults-repo')).toBeDefined()
  })

  it('derives the label with the DB-042 rule (upper-case first, 6 code points)', async () => {
    const result = await commands.projectAddManual('/home/dev/src/straße', null)
    expect(result.project?.name).toBe('straße')
    expect(result.project?.label).toBe('STRASS')
  })

  it('picks the least-used palette color among active projects (D-29)', async () => {
    // Whatever the fixture world wears, no active color may be used FEWER
    // times than the one the mock chose (ties break by palette order).
    const before = mockState().projects.filter((p) => p.state === 'active')
    const counts = new Map<string, number>()
    for (const row of before) counts.set(row.color, (counts.get(row.color) ?? 0) + 1)
    const result = await commands.projectAddManual('/home/dev/src/color-repo', null)
    const chosen = result.project?.color
    expect(chosen).toBeDefined()
    const chosenCount = counts.get(chosen ?? '') ?? 0
    for (const [color, count] of counts) {
      if (count < chosenCount) {
        throw new Error(
          `color ${color} is used ${count}x but ${chosen} (${chosenCount}x) was chosen`,
        )
      }
    }
  })

  it('leaves the store and the journal untouched on the duplicate arm', async () => {
    const [existing] = await commands.projectsList(null)
    const before = mockState()
    const beforeProjects = before.projects.length
    const beforeJournal = before.activity.length
    await commands.projectAddManual(pathOf(existing?.name ?? ''), null)
    expect(mockState().projects).toHaveLength(beforeProjects)
    expect(mockState().activity).toHaveLength(beforeJournal)
  })

  it('answers duplicate_active for a path an active project already owns', async () => {
    const [existing] = await commands.projectsList(null)
    const result = await commands.projectAddManual(pathOf(existing?.name ?? ''), null)
    expect(result.outcome).toBe('duplicate_active')
    expect(result.project).toBeNull()
    expect(result.existing?.id).toBe(existing?.id)
    expect(result.existing?.state).toBe('active')
  })

  it('answers archived_duplicate for a path an archived project owns', async () => {
    const all = await commands.projectsList(true)
    const archived = all.find((p) => p.state === 'archived')
    expect(archived).toBeDefined()
    const result = await commands.projectAddManual(pathOf(archived?.name ?? ''), null)
    expect(result.outcome).toBe('archived_duplicate')
    expect(result.existing?.state).toBe('archived')
  })

  it('rejects an empty path with E_VALIDATION', async () => {
    await expect(commands.projectAddManual('   ', null)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'path' },
    })
  })

  it('journals the add where activity_list can read it', async () => {
    const added = await commands.projectAddManual('/home/dev/src/journaled-repo', null)
    const page = await commands.activityList({})
    const entry = page.entries.find(
      (e) => e.action === 'project.added' && e.entity_id === added.project?.id,
    )
    expect(entry).toBeDefined()
    expect(entry?.visibility).toBe('user')
    expect(entry?.result).toBe('ok')
  })

  /** The mock row path is derived from the fixture name; read it via state. */
  function pathOf(name: string): string {
    const row = mockState().projects.find((p) => p.name === name)
    expect(row, `a fixture project named ${name}`).toBeDefined()
    return row?.path ?? ''
  }
})

describe('projectArchive', () => {
  it('archives a scan-born row, serves the exclusion back, and journals it', async () => {
    // T-108/D-194's happy path over the mock store: the row flips
    // (readable behind the include flag), the scan-born path joins the
    // exclusions (readable through the screen's own list), and the one
    // journal entry lands where `activityList` reads it.
    const [target] = mockState().projects.filter((row) => row.state === 'active')
    expect(target).toBeDefined()
    const exclusionsBefore = (await commands.exclusionsList()).length

    await expect(commands.projectArchive(target?.id ?? '')).resolves.toBeNull()

    const columns = await commands.projectsList(true)
    const archived = columns.find((column) => column.id === target?.id)
    expect(archived?.state).toBe('archived')
    const exclusions = await commands.exclusionsList()
    expect(exclusions).toHaveLength(exclusionsBefore + 1)
    const written = exclusions.find((row) => row.path_key === pathKeyOf(target?.path ?? ''))
    expect(written?.path).toBe(target?.path)
    expect(written?.reason).toBe('removed_scanned')
    const page = await commands.activityList({ actions: ['project.archived'] })
    const entry = page.entries.find((e) => e.entity_id === target?.id)
    expect(entry, 'one project.archived entry naming the row').toBeDefined()
    expect(entry?.visibility).toBe('user')
    expect(entry?.result).toBe('ok')
    expect(entry?.project_id).toBe(target?.id)
  })

  it('archives a manual-born row without an exclusion row', async () => {
    const added = await commands.projectAddManual('/home/dev/src/manual-born', null)
    expect(added.outcome).toBe('added')
    const exclusionsBefore = (await commands.exclusionsList()).length

    await expect(commands.projectArchive(added.project?.id ?? '')).resolves.toBeNull()

    expect(await commands.exclusionsList()).toHaveLength(exclusionsBefore)
    const columns = await commands.projectsList(true)
    expect(columns.find((column) => column.id === added.project?.id)?.state).toBe('archived')
  })

  it('answers a re-send on an archived row as a no-op, journaling nothing', async () => {
    const [target] = mockState().projects.filter((row) => row.state === 'active')
    await commands.projectArchive(target?.id ?? '')
    const journalLength = mockState().activity.length
    const exclusionsAfterFirst = (await commands.exclusionsList()).length

    await expect(commands.projectArchive(target?.id ?? '')).resolves.toBeNull()

    expect(mockState().activity).toHaveLength(journalLength)
    expect(await commands.exclusionsList()).toHaveLength(exclusionsAfterFirst)
    const columns = await commands.projectsList(true)
    expect(columns.find((column) => column.id === target?.id)?.state).toBe('archived')
  })

  it('rejects an unknown id with E_VALIDATION, changing nothing', async () => {
    const journalLength = mockState().activity.length
    const exclusionsBefore = (await commands.exclusionsList()).length

    await expect(commands.projectArchive('no-such-row')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'project_id',
        value: 'no-such-row',
        reason: 'no project has this id',
      },
    })

    // D-115's cap, pinned at the boundary: an over-long id echoes
    // truncated with the suffix, never whole (the mock mirrors the
    // Rust `bounded_echo` the real refusal uses).
    const longId = 'x'.repeat(70)
    await expect(commands.projectArchive(longId)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'project_id',
        value: `${'x'.repeat(64)} (+6 more code points)`,
        reason: 'no project has this id',
      },
    })

    expect(mockState().activity).toHaveLength(journalLength)
    expect(await commands.exclusionsList()).toHaveLength(exclusionsBefore)
  })

  it('serves a just-archived row with the row-decidable clean halves, whatever its flags said', async () => {
    // The mock's read mirrors production's verdict functions (round 2's
    // P2-1 + round 1's P3-4): an archived column never shows the dirty
    // dot (DB-014), the §5.7 watch markings or the SYS-010 notice. The
    // row is deliberately loaded with every marking — including the
    // mutually-exclusive pair, which only a hand-edited row can hold —
    // precisely because the archived read must not trust any of them;
    // the row's own flags stay untouched for the store-side assertions.
    const target = mockState().projects.find((row) => row.state === 'active')
    expect(target, 'an active fixture row to archive').toBeDefined()
    if (!target) return
    target.dirty = true
    target.externalChanged = true
    target.fileDeleted = true
    target.unwatchedReason = 'gone'

    await expect(commands.projectArchive(target.id)).resolves.toBeNull()

    const columns = await commands.projectsList(true)
    const archived = columns.find((column) => column.id === target.id)
    expect(archived?.state).toBe('archived')
    expect(archived?.dirty).toBe(false)
    expect(archived?.external_changed).toBe(false)
    expect(archived?.file_deleted).toBe(false)
    expect(archived?.unwatched_reason).toBeNull()
    const stored = mockState().projects.find((row) => row.id === target.id)
    expect(stored?.dirty).toBe(true)
    expect(stored?.externalChanged).toBe(true)
    expect(stored?.fileDeleted).toBe(true)
    expect(stored?.unwatchedReason).toBe('gone')
  })

  it('converges over an existing exclusion row of the same path key', async () => {
    // D-194's convergence arm: an already-excluded path stays as it is
    // — the existing row keeps its own reason, and no duplicate lands.
    const [target] = mockState().projects.filter((row) => row.state === 'active')
    const key = pathKeyOf(target?.path ?? '')
    mockState().exclusions.push({
      id: 'pre-seeded-exclusion',
      path: target?.path ?? '',
      path_key: key,
      reason: 'manual',
      created_at: '2026-01-01T00:00:00.000Z',
    })

    await expect(commands.projectArchive(target?.id ?? '')).resolves.toBeNull()

    const exclusions = await commands.exclusionsList()
    const forThePath = exclusions.filter((row) => row.path_key === key)
    expect(forThePath).toHaveLength(1)
    expect(forThePath[0]?.reason).toBe('manual')
    expect(forThePath[0]?.id).toBe('pre-seeded-exclusion')
  })
})

describe('projectBuildPreview (T-358, MAIN-033)', () => {
  /** The first two ACTIVE project rows — the mock store's own rows are
   * the seed; each test pins their write settings. */
  const activeRows = (): MockProjectRow[] => {
    const rows = mockState()
      .projects.filter((row) => row.state === 'active')
      .sort((a, b) => a.sortOrder - b.sortOrder)
    if (rows.length < 2) throw new Error('two active projects must exist')
    return rows
  }

  /** A deterministic one-heading library: `## Style` over one active
   * instruction whose Main carries a two-paragraph body (the fixture
   * world's shapes are rng-driven; the preview's expected text is a
   * literal, so the world must be one). */
  const seedLibrary = (state: ReturnType<typeof mockState>): void => {
    const at = '2026-11-27T10:00:00.000Z'
    state.library = [
      {
        kind: 'heading',
        id: 'heading-style',
        position: 1,
        headingLevel: 2,
        title: 'Style',
        state: 'active',
        createdAt: at,
        updatedAt: at,
      },
      {
        kind: 'instruction',
        id: 'instruction-brief',
        position: 2,
        title: 'Be brief',
        state: 'active',
        variants: [
          {
            id: 'variant-main',
            name: 'Main',
            content: 'Be brief.\n\nRun the checks.',
            isMain: true,
            state: 'active',
            createdAt: at,
            updatedAt: at,
            deletedAt: null,
          },
        ],
        createdAt: at,
        updatedAt: at,
      },
    ]
    state.assignments = []
  }

  it("composes scheme A's managed text and answers the target path", async () => {
    // ALG-030/020…023 over the mock rows: the enabled reading line
    // splices around the assigned cell's normalized text (the heading
    // by ALG-020, the extend cell composed per ALG-023), the blocks
    // joined by exactly one blank line — the same text the backend
    // command builds from the same-shaped state (the real-sync
    // oracle lives in the Rust battery; here the literals are the
    // oracle).
    const state = mockState()
    const [row] = activeRows()
    seedLibrary(state)
    row.scheme = 'A'
    row.baseName = 'AGENTS.md'
    row.extraProjectEnabled = true
    row.extraProjectPosition = 'start'
    row.extraLocalEnabled = false
    const extended = await commands.assignmentSetCustom(
      row.id,
      'instruction-brief',
      'plus more\r\n',
      'extend',
    )
    expect(extended?.target).toEqual({
      kind: 'extend',
      value: { base: 'variant-main', supplement: 'plus more\n' },
    })
    // The write command above journaled; the preview (a read) must add
    // nothing to that.
    const journalBefore = state.activity.length

    const preview = await commands.projectBuildPreview(row.id)
    expect(preview.target_path).toBe(`${row.path}/AGENTS.md`)
    expect(preview.text).toBe(
      'If AGENTS.project.md exists in the repository root, read it before starting \
work and follow its instructions.\n\
\n\
## Style\n\
\n\
Be brief.\n\
\n\
Run the checks.\n\
\n\
plus more',
    )
    expect(preview.text).not.toContain('Created by EunomiaSync')
    expect(state.activity).toHaveLength(journalBefore)
  })

  it('answers the managed block only for scheme B, ignoring the reading lines', async () => {
    const state = mockState()
    const [row] = activeRows()
    seedLibrary(state)
    row.scheme = 'B'
    row.baseName = 'CLAUDE.md'
    row.extraProjectEnabled = true
    row.extraProjectPosition = 'end'
    row.extraLocalEnabled = true
    row.extraLocalPosition = 'end'
    await commands.assignmentSet(row.id, 'instruction-brief', true, 'variant-main')

    const preview = await commands.projectBuildPreview(row.id)
    expect(preview.target_path).toBe(`${row.path}/CLAUDE.md`)
    expect(preview.text).toBe('## Style\n\nBe brief.\n\nRun the checks.')
    expect(preview.text).not.toContain('read it before starting work')
  })

  it('answers the scheme-C project path and empty text for an empty column', async () => {
    const state = mockState()
    const [row] = activeRows()
    seedLibrary(state)
    row.scheme = 'C'
    row.baseName = 'AGENTS.md'
    row.extraProjectEnabled = false
    row.extraLocalEnabled = false
    // A column with no assignments: the empty text (MAIN-033's note
    // is the modal's own), the path still answered.
    const preview = await commands.projectBuildPreview(row.id)
    expect(preview.target_path).toBe(`${row.path}/AGENTS.project.md`)
    expect(preview.text).toBe('')
  })

  it('normalizes the cells texts per ALG-002 before joining', async () => {
    // The ALG-002 twin: a stored CRLF body with edge whitespace and a
    // BOM builds the normalized block — the leading empty lines, the
    // BOM and the trailing whitespace go, while the first CONTENT
    // line's own leading spaces survive (ALG-002's rule: they are
    // inside the text from there on).
    const state = mockState()
    const [row] = activeRows()
    seedLibrary(state)
    row.scheme = 'C'
    const brief = state.library[1]
    if (brief?.kind !== 'instruction') throw new Error('the seeded instruction must exist')
    brief.variants[0]!.content = '\u{FEFF}\r\n\r\n  Be brief.  \r\n\n'
    await commands.assignmentSet(row.id, 'instruction-brief', true, 'variant-main')

    const preview = await commands.projectBuildPreview(row.id)
    expect(preview.text).toBe('## Style\n\n  Be brief.')
  })

  it('refuses an archived or unknown id with the project_update envelope', async () => {
    const state = mockState()
    const archived = state.projects.find((row) => row.state === 'archived')
    if (!archived) throw new Error('the fixture world must hold an archived project')
    const journalBefore = state.activity.length
    for (const id of [archived.id, 'no-such-project']) {
      await expect(commands.projectBuildPreview(id)).rejects.toMatchObject({
        code: 'E_VALIDATION',
        params: {
          field: 'project_id',
          value: id,
          reason: 'no active project has this id',
        },
      })
    }
    // D-115's cap, the bounded echo.
    const longId = 'y'.repeat(70)
    await expect(commands.projectBuildPreview(longId)).rejects.toMatchObject({
      params: { value: `${'y'.repeat(64)} (+6 more code points)` },
    })
    expect(state.activity).toHaveLength(journalBefore)
  })
})

describe('projectRestore', () => {
  it('restores an archived row, removes the exclusion, and journals it', async () => {
    // T-109/D-205's happy path over the mock store: the row flips
    // back (readable through the include flag), the archive-born
    // exclusion row is gone (readable through the screen's own list),
    // and the one journal entry lands where `activityList` reads it.
    // The round trip is the user's own: archive first (the real
    // command's twin), then restore.
    const target = mockState().projects.find((row) => row.state === 'active')
    expect(target, 'an active fixture row to archive').toBeDefined()
    if (!target) return
    await commands.projectArchive(target.id)
    expect(
      (await commands.exclusionsList()).some((row) => row.path_key === pathKeyOf(target.path)),
      'the archive wrote the exclusion row',
    ).toBe(true)

    await expect(commands.projectRestore(target.id)).resolves.toBeNull()

    const columns = await commands.projectsList(true)
    expect(columns.find((column) => column.id === target.id)?.state).toBe('active')
    expect(
      (await commands.exclusionsList()).filter((row) => row.path_key === pathKeyOf(target.path)),
    ).toHaveLength(0)
    const page = await commands.activityList({ actions: ['project.restored'] })
    const entry = page.entries.find((e) => e.entity_id === target.id)
    expect(entry, 'one project.restored entry naming the row').toBeDefined()
    expect(entry?.visibility).toBe('user')
    expect(entry?.result).toBe('ok')
    expect(entry?.project_id).toBe(target.id)
  })

  it('deletes a pre-existing manual exclusion the archive left in place', async () => {
    // The pinned owner ruling (T-108's card comment, D-194 round-2
    // P3-6, D-205 point 1): the archive converges over a manual
    // exclusion of the same path key and leaves it; the restore
    // deletes it — reason-agnostic.
    const target = mockState().projects.find((row) => row.state === 'active')
    expect(target, 'an active fixture row to archive').toBeDefined()
    if (!target) return
    const key = pathKeyOf(target.path)
    mockState().exclusions.push({
      id: 'pre-existing-manual-exclusion',
      path: target.path,
      path_key: key,
      reason: 'manual',
      created_at: '2026-01-01T00:00:00.000Z',
    })
    await commands.projectArchive(target.id)

    await expect(commands.projectRestore(target.id)).resolves.toBeNull()

    expect((await commands.exclusionsList()).filter((row) => row.path_key === key)).toHaveLength(0)
  })

  it('answers a re-send on an active row as a no-op, journaling nothing', async () => {
    // D-205 point 3, the D-194 point-2 mirror: the converging re-send
    // writes and journals nothing, and does not churn an exclusion
    // that appeared after the first restore.
    const target = mockState().projects.find((row) => row.state === 'archived')
    expect(target, 'an archived fixture row to restore').toBeDefined()
    if (!target) return
    await expect(commands.projectRestore(target.id)).resolves.toBeNull()
    const journalLength = mockState().activity.length
    const key = pathKeyOf(target.path)
    mockState().exclusions.push({
      id: 'post-restore-exclusion',
      path: target.path,
      path_key: key,
      reason: 'manual',
      created_at: '2026-02-01T00:00:00.000Z',
    })
    const exclusionsAfterFirst = (await commands.exclusionsList()).length

    await expect(commands.projectRestore(target.id)).resolves.toBeNull()

    expect(mockState().activity).toHaveLength(journalLength)
    expect(await commands.exclusionsList()).toHaveLength(exclusionsAfterFirst)
    const columns = await commands.projectsList(true)
    expect(columns.find((column) => column.id === target.id)?.state).toBe('active')
  })

  it('rejects an unknown id with E_VALIDATION, bounded echo included', async () => {
    // The loaded fixture world, snapshotted before the act: the
    // arrays are compared whole after the refusals, so any write the
    // command wrongly made (not just a length change) reddens.
    const journalBefore = mockState().activity.map((entry) => ({ ...entry }))
    const exclusionsBefore = (await commands.exclusionsList()).map((row) => ({ ...row }))

    await expect(commands.projectRestore('no-such-row')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'project_id',
        value: 'no-such-row',
        reason: 'no project has this id',
      },
    })

    // D-115's cap, pinned at the boundary: an over-long id echoes
    // truncated with the suffix, never whole (the mock mirrors the
    // Rust `bounded_echo` the real refusal uses).
    const longId = 'x'.repeat(70)
    await expect(commands.projectRestore(longId)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'project_id',
        value: `${'x'.repeat(64)} (+6 more code points)`,
        reason: 'no project has this id',
      },
    })

    // Nothing was written: the loaded fixture world stands as it was
    // (the arrays themselves, not just their lengths — a swap would
    // fail; round 1's P3-2, the mock twin of the Rust-side fix).
    expect(mockState().activity).toEqual(journalBefore)
    expect(await commands.exclusionsList()).toEqual(exclusionsBefore)
  })
})

describe('projectRelink', () => {
  /** An active row to relink, straight from the store the command serves. */
  function activeRow(): MockProjectRow | undefined {
    return mockState().projects.find((row) => row.state === 'active')
  }

  it('relinks the row, clears the marking, and journals it', async () => {
    // T-110/D-208's happy path over the mock store: the row's path
    // moves to the picked location, the D-09 marking clears (the
    // freshly located folder is a live repository — the modelled
    // half; production probes fresh on every read), the assignments
    // stay keyed by the unmoved id (the DoD's second half), the
    // position and name stay, and the one journal entry lands where
    // `activityList` reads it.
    const target = activeRow()
    expect(target, 'an active fixture row to relink').toBeDefined()
    if (!target) return
    const assignmentsBefore = mockState()
      .assignments.filter((assignment) => assignment.projectId === target.id)
      .map((assignment) => ({ ...assignment }))
    target.available = false
    target.unavailableReason = 'missing'
    const picked = '/moved/repo-one'

    await expect(commands.projectRelink(target.id, picked)).resolves.toBeNull()

    expect(target.path).toBe(picked)
    expect(target.available, 'the D-09 DoD: available again').toBe(true)
    expect(target.unavailableReason).toBeNull()
    expect(target.sortOrder).toBeGreaterThan(0)
    const columns = await commands.projectsList(true)
    expect(columns.find((column) => column.id === target.id)?.available).toBe(true)
    expect(
      mockState().assignments.filter((assignment) => assignment.projectId === target.id),
    ).toEqual(assignmentsBefore)
    const page = await commands.activityList({ actions: ['project.changed'] })
    const entry = page.entries.find((e) => e.entity_id === target.id)
    expect(entry, 'one project.changed entry naming the row').toBeDefined()
    expect(entry?.visibility).toBe('user')
    expect(entry?.result).toBe('ok')
    expect(entry?.project_id).toBe(target.id)
  })

  it("answers a re-send of the row's own location as a no-op", async () => {
    // D-208 point 3, the mock twin of the Rust converging arm: a
    // re-send of the location the row already holds writes and
    // journals nothing.
    const target = activeRow()
    expect(target, 'an active fixture row to relink').toBeDefined()
    if (!target) return
    const picked = '/moved/repo-two'
    await commands.projectRelink(target.id, picked)
    const journalLength = mockState().activity.length
    const rowSnapshot = { ...target }

    await expect(commands.projectRelink(target.id, picked)).resolves.toBeNull()

    expect(mockState().activity).toHaveLength(journalLength)
    expect({ ...target }).toEqual(rowSnapshot)
  })

  it('refuses an unknown id with E_VALIDATION, bounded echo included', async () => {
    // The loaded fixture world, snapshotted before the act: the
    // arrays are compared whole after the refusals, so any write the
    // command wrongly made (not just a length change) reddens.
    const projectsBefore = mockState().projects.map((row) => ({ ...row }))
    const journalBefore = mockState().activity.map((entry) => ({ ...entry }))
    const exclusionsBefore = (await commands.exclusionsList()).map((row) => ({ ...row }))

    await expect(commands.projectRelink('no-such-row', '/moved/repo')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'project_id',
        value: 'no-such-row',
        reason: 'no active project has this id',
      },
    })

    // D-115's cap, pinned at the boundary: an over-long id echoes
    // truncated with the suffix, never whole (the mock mirrors the
    // Rust `bounded_echo` the real refusal uses).
    const longId = 'x'.repeat(70)
    await expect(commands.projectRelink(longId, '/moved/repo')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'project_id',
        value: `${'x'.repeat(64)} (+6 more code points)`,
        reason: 'no active project has this id',
      },
    })

    // Nothing was written: the loaded fixture world stands as it was.
    expect(mockState().projects).toEqual(projectsBefore)
    expect(mockState().activity).toEqual(journalBefore)
    expect(await commands.exclusionsList()).toEqual(exclusionsBefore)
  })

  it('refuses an archived id as the stale screen shape', async () => {
    // D-208 point 3's id filter (the `projectUpdate` mirror): the
    // Locate action rides an active column, so an archived id is the
    // same stale-screen refusal.
    const archived = mockState().projects.find((row) => row.state === 'archived')
    expect(archived, 'an archived fixture row').toBeDefined()
    if (!archived) return

    await expect(commands.projectRelink(archived.id, '/moved/repo')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'project_id',
        reason: 'no active project has this id',
      },
    })
  })

  it("refuses an empty path with the real gate's own refusal shape", async () => {
    // The empty-path arm mirrors the real command's answer (the
    // SEC-003 gate refuses the unresolvable/never-picked string
    // before any probe — round 1's P3-3: the legacy
    // `validation.path_required` key was the mock's invention, never
    // a backend envelope).
    const target = activeRow()
    expect(target, 'an active fixture row to relink').toBeDefined()
    if (!target) return

    await expect(commands.projectRelink(target.id, '   ')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'path',
        reason:
          'must be a registered project or a folder picked in the dialog this session (SEC-003)',
      },
    })
  })

  it('refuses a path an active row holds, writing nothing', async () => {
    // DB-015's active half (D-208 point 5): the resolved key belongs
    // to another ACTIVE project — the typed refusal naming `path`,
    // no offer id, and the world untouched.
    const actives = mockState().projects.filter((row) => row.state === 'active')
    expect(actives.length, 'two active fixture rows to collide').toBeGreaterThanOrEqual(2)
    const [occupier, target] = actives
    const projectsBefore = mockState().projects.map((row) => ({ ...row }))

    await expect(commands.projectRelink(target.id, occupier.path)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'path',
        reason: 'an active project already has this path (DB-015)',
      },
    })

    expect(mockState().projects).toEqual(projectsBefore)
  })

  it('refuses a path an archived row holds, carrying the restore offer', async () => {
    // DB-015's archived half (D-208 point 5, the pinned cross-link's
    // territory): the refusal carries `existing_project_id` — the
    // archived row's id, exactly what `projectRestore` consumes — and
    // the world untouched.
    const target = activeRow()
    expect(target, 'an active fixture row to relink').toBeDefined()
    if (!target) return
    const archived = mockState().projects.find((row) => row.state === 'archived')
    expect(archived, 'an archived fixture row as the occupier').toBeDefined()
    if (!archived) return
    const projectsBefore = mockState().projects.map((row) => ({ ...row }))

    await expect(commands.projectRelink(target.id, archived.path)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'path',
        reason: 'an archived project already has this path (DB-015)',
        existing_project_id: archived.id,
      },
    })

    expect(mockState().projects).toEqual(projectsBefore)
  })
})

describe('projectUpdate', () => {
  /**
   * An active row to edit, straight from the store the command serves.
   * Callers snapshot (`{ ...row }`) BEFORE the act: the store mutates
   * rows in place, so comparing the live object with itself can never
   * fail (round 1's P3-5).
   */
  function activeRow(): MockProjectRow {
    const row = mockState().projects.find((candidate) => candidate.state === 'active')
    expect(row, 'an active fixture row to edit').toBeDefined()
    return row as MockProjectRow
  }

  /** The mock's journal, read through the screen's own list. */
  async function changedEntries(): Promise<ActivityEntryWire[]> {
    const page = await commands.activityList({ actions: ['project.changed'], limit: 500 })
    return page.entries
  }

  it('saves the four fields and journals one attributable entry', async () => {
    // T-106/D-201's happy path: every §6.9 boundary-legal field saved,
    // the write readable back through `projectsList` (name, label,
    // color — the columns the list serves), and one `project.changed`
    // entry where `activityList` reads it. The row is snapshotted
    // first: the immutable-half asserts must compare against the world
    // as it stood, not the object the command just mutated.
    const row = activeRow()
    const before = { ...row }
    const journalBefore = (await changedEntries()).length

    await expect(
      commands.projectUpdate(
        row.id,
        'x'.repeat(100),
        'L'.repeat(6),
        'teal',
        'd'.repeat(1000),
        null,
        null,
        null,
        null,
      ),
    ).resolves.toBeNull()

    const stored = mockState().projects.find((candidate) => candidate.id === row.id)
    expect(stored?.name).toBe('x'.repeat(100))
    expect(stored?.label).toBe('L'.repeat(6))
    expect(stored?.color).toBe('teal')
    expect(stored?.description).toBe('d'.repeat(1000))
    const columns = await commands.projectsList(true)
    const column = columns.find((entry) => entry.id === row.id)
    expect(column?.name).toBe('x'.repeat(100))
    expect(column?.label).toBe('L'.repeat(6))
    expect(column?.color).toBe('teal')
    // The immutable half (PRJ-008): the path and the origin stay.
    expect(stored?.path).toBe(before.path)
    expect(stored?.addedVia).toBe(before.addedVia)
    const journal = await changedEntries()
    expect(journal).toHaveLength(journalBefore + 1)
    // `activityList` serves newest-first, so the NEW entry is index 0
    // (round 1's P3-5: the last index is the oldest).
    const entry = journal[0]
    expect(entry?.action).toBe('project.changed')
    expect(entry?.entity_type).toBe('project')
    expect(entry?.entity_id).toBe(row.id)
    expect(entry?.project_id).toBe(row.id)
    expect(entry?.visibility).toBe('user')
    expect(entry?.result).toBe('ok')
    expect(entry?.details).toBeNull()
  })

  it('rejects each invalid field with E_VALIDATION, changing nothing', async () => {
    // The card's DoD: an invalid field refuses the whole save — the
    // row stays verbatim and the journal claims nothing. One case per
    // §6.9 field rule, the Rust battery's mirror.
    const row = activeRow()
    const before = { ...row }
    const journalBefore = (await changedEntries()).length
    const cases: Array<[string, string | null, string | null, string | null, string | null]> = [
      ['name', 'x'.repeat(101), 'NEWL', 'teal', 'new description'],
      ['label', 'New Name', 'ABCDEFG', 'teal', 'new description'],
      ['description', 'New Name', 'NEWL', 'teal', 'd'.repeat(1001)],
      ['color', 'New Name', 'NEWL', 'purple', 'new description'],
    ]
    for (const [field, name, label, color, description] of cases) {
      await expect(
        commands.projectUpdate(row.id, name, label, color, description, null, null, null, null),
      ).rejects.toMatchObject({
        code: 'E_VALIDATION',
        params: { field },
      })
    }
    const stored = mockState().projects.find((candidate) => candidate.id === row.id)
    expect(stored).toEqual(before)
    expect((await changedEntries()).length).toBe(journalBefore)
  })

  it('echoes no value in the description refusal (SEC-008, round 1 P3-6)', async () => {
    // The description is the one field whose refusal carries NO value:
    // stored prose never rides the envelope (the Rust command's
    // variant-text rule, mirrored). The rejection is captured with its
    // honest wire type so the absence itself is asserted.
    const row = activeRow()
    const before = { ...row }
    const rejection = await commands
      .projectUpdate(row.id, 'New Name', 'NEWL', 'teal', 'd'.repeat(1001), null, null, null, null)
      .then(
        () => undefined,
        (error: AppErrorWire) => error,
      )
    expect(rejection?.code).toBe('E_VALIDATION')
    expect(rejection?.params?.field).toBe('description')
    expect(rejection?.params?.value).toBeUndefined()
    const stored = mockState().projects.find((candidate) => candidate.id === row.id)
    expect(stored).toEqual(before)
  })

  it('answers the field validation before the id, in table order', async () => {
    // Round 1's P2-4 mirror: the fields validate before the row lookup
    // (an unknown id plus an invalid field answers about the FIELD),
    // and the §6.9 table order decides which invalid field answers.
    await expect(
      commands.projectUpdate(
        'no-such-row',
        'x'.repeat(101),
        null,
        'purple',
        null,
        null,
        null,
        null,
        null,
      ),
    ).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'name',
        value: `${'x'.repeat(64)} (+37 more code points)`,
      },
    })
  })

  it('answers the middle fields in table order too (round 2 P3-3)', async () => {
    // The §6.9 table order's MIDDLE pair: an invalid label and an
    // over-long description together answer about the label (the
    // earlier of the two) — the first-vs-last pin above cannot catch
    // a label/description swap.
    const row = activeRow()
    await expect(
      commands.projectUpdate(
        row.id,
        'New Name',
        'ABCDEFG',
        null,
        'd'.repeat(1001),
        null,
        null,
        null,
        null,
      ),
    ).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'label' },
    })
  })

  it('mirrors the label control-character refusal text exactly (round 2 P3-1)', async () => {
    // The reason string is the Rust Display's own text — the char
    // formatted the way Rust's `{:?}` Debug formats it ('\u{7}'),
    // never JSON.stringify's "\u0007" — so a UI test written against
    // the mock asserts a string the backend actually sends.
    const row = activeRow()
    const rejection = await commands
      .projectUpdate(row.id, null, 'a\u0007b', null, null, null, null, null, null)
      .then(
        () => undefined,
        (error: AppErrorWire) => error,
      )
    expect(rejection?.code).toBe('E_VALIDATION')
    expect(rejection?.params?.field).toBe('label')
    expect(rejection?.params?.reason).toBe(
      "project label contains the control character '\\u{7}' at index 1",
    )
  })

  it('treats U+FEFF as the visible character Rust treats it as (round 2 P3-2)', async () => {
    // JS `/\s` and `String.trim()` treat the BOM (U+FEFF, a Cf char)
    // as whitespace; Rust's `char::is_whitespace` does not — the
    // char is VISIBLE, so a BOM-only label is a legal 1-character
    // label and a BOM-fringed name keeps its inner text verbatim.
    // The mock's validators use Rust's White_Space set, so both hold
    // here exactly as the core validators hold them.
    const row = activeRow()
    const before = { ...row }

    await expect(
      commands.projectUpdate(row.id, null, '\uFEFF', null, null, null, null, null, null),
    ).resolves.toBeNull()

    const stored = mockState().projects.find((candidate) => candidate.id === row.id)
    expect(stored?.label).toBe('\uFEFF')
    expect(stored).toEqual({ ...before, label: '\uFEFF' })
  })

  it('leaves absent fields unchanged (the patch semantics)', async () => {
    const row = activeRow()
    const before = { ...row }

    await expect(
      commands.projectUpdate(row.id, null, 'ONLY', null, null, null, null, null, null),
    ).resolves.toBeNull()

    const stored = mockState().projects.find((candidate) => candidate.id === row.id)
    expect(stored?.label).toBe('ONLY')
    expect(stored?.name).toBe(before.name)
    expect(stored?.color).toBe(before.color)
    expect(stored?.description).toBe(before.description)
  })

  it('answers an unchanged resend as a no-op, journaling nothing', async () => {
    // The verified no-op (D-188/D-201): a re-send of the stored values
    // writes and journals nothing. The snapshot is the oracle — the
    // live row object would compare with itself.
    const row = activeRow()
    const before = { ...row }
    const journalBefore = (await changedEntries()).length

    await expect(
      commands.projectUpdate(
        row.id,
        row.name,
        row.label,
        row.color,
        row.description,
        null,
        null,
        null,
        null,
      ),
    ).resolves.toBeNull()

    const stored = mockState().projects.find((candidate) => candidate.id === row.id)
    expect(stored).toEqual(before)
    expect((await changedEntries()).length).toBe(journalBefore)
  })

  it('answers the all-null send as a no-op too', async () => {
    // D-201 point 5's other no-op arm: every field `null` — an empty
    // patch, nothing written, nothing journaled.
    const row = activeRow()
    const before = { ...row }
    const journalBefore = (await changedEntries()).length

    await expect(
      commands.projectUpdate(row.id, null, null, null, null, null, null, null, null),
    ).resolves.toBeNull()

    const stored = mockState().projects.find((candidate) => candidate.id === row.id)
    expect(stored).toEqual(before)
    expect((await changedEntries()).length).toBe(journalBefore)
  })

  it('rejects an empty name, saving nothing (the 6.9 lower bound)', async () => {
    const row = activeRow()
    const before = { ...row }

    await expect(
      commands.projectUpdate(row.id, '   ', null, null, null, null, null, null, null),
    ).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'name' },
    })

    const stored = mockState().projects.find((candidate) => candidate.id === row.id)
    expect(stored).toEqual(before)
  })

  it('saves an empty description as a real change (6.9: empty is legal)', async () => {
    // The fixture rows seed the column default `''`, so the row's
    // description is set directly first (the §5.7-marking posture) —
    // clearing prose is a REAL change, journaled like any other, not
    // the no-op a null send would be.
    const row = activeRow()
    row.description = 'seeded prose'
    const journalBefore = (await changedEntries()).length

    await expect(
      commands.projectUpdate(row.id, null, null, null, '', null, null, null, null),
    ).resolves.toBeNull()

    const stored = mockState().projects.find((candidate) => candidate.id === row.id)
    expect(stored?.description).toBe('')
    expect((await changedEntries()).length).toBe(journalBefore + 1)
  })

  it('saves a description with a NUL stripped (6.9)', async () => {
    const row = activeRow()

    await expect(
      commands.projectUpdate(
        row.id,
        null,
        null,
        null,
        'kept text\0stripped byte',
        null,
        null,
        null,
        null,
      ),
    ).resolves.toBeNull()

    const stored = mockState().projects.find((candidate) => candidate.id === row.id)
    expect(stored?.description).toBe('kept textstripped byte')
  })

  it('saves a duplicate name without refusal (6.9 warning case)', async () => {
    // §6.9's "Duplicates are allowed with a warning": the duplicate
    // saves — the warning is the modal's live concern, never a refusal.
    // The generator's index suffix keeps the fixture names unique, so
    // this send is a real change, not the verified no-op.
    const [first, second] = mockState().projects.filter((candidate) => candidate.state === 'active')
    expect(second).toBeDefined()
    expect(first?.name).not.toBe(second?.name)
    const journalBefore = (await changedEntries()).length

    await expect(
      commands.projectUpdate(second.id, first.name, null, null, null, null, null, null, null),
    ).resolves.toBeNull()

    const stored = mockState().projects.find((candidate) => candidate.id === second?.id)
    expect(stored?.name).toBe(first?.name)
    const untouched = mockState().projects.find((candidate) => candidate.id === first?.id)
    expect(untouched?.name).toBe(first?.name)
    expect((await changedEntries()).length).toBe(journalBefore + 1)
  })

  it('rejects an unknown or archived id with E_VALIDATION, bounded echo included', async () => {
    const journalBefore = (await changedEntries()).length

    await expect(
      commands.projectUpdate('no-such-row', 'New Name', null, null, null, null, null, null, null),
    ).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'project_id',
        value: 'no-such-row',
        reason: 'no active project has this id',
      },
    })
    // D-115's cap, pinned at the boundary: an over-long id echoes
    // truncated with the suffix, never whole.
    const longId = 'x'.repeat(70)
    await expect(
      commands.projectUpdate(longId, null, null, null, null, null, null, null, null),
    ).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'project_id',
        value: `${'x'.repeat(64)} (+6 more code points)`,
        reason: 'no active project has this id',
      },
    })
    // The archived row answers the same refusal (D-201 point 4): the
    // modal opens from an active column, so a row archived since the
    // modal opened is a stale screen.
    const archived = mockState().projects.find((candidate) => candidate.state === 'archived')
    expect(archived).toBeDefined()
    await expect(
      commands.projectUpdate(
        archived?.id ?? '',
        'New Name',
        null,
        null,
        null,
        null,
        null,
        null,
        null,
      ),
    ).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'project_id',
        reason: 'no active project has this id',
      },
    })
    expect((await changedEntries()).length).toBe(journalBefore)
  })

  it('saves the scheme, file name and extra files, and journals one attributable entry', async () => {
    // T-107/D-204's happy path: the whole PRJ-012 half saved by one
    // call — the scheme, the file name, and one extra file enabled at
    // start (the other re-sent at its stored value: the patch's
    // equal-strip on the new half) — readable back through the row,
    // with the T-106 fields and the immutable half untouched and one
    // `project.changed` entry where `activityList` reads it (no
    // details — the row holds the facts, D-204 point 8).
    const row = activeRow()
    const before = { ...row }
    const journalBefore = (await changedEntries()).length

    await expect(
      commands.projectUpdate(
        row.id,
        null,
        null,
        null,
        null,
        'B',
        'CLAUDE.md',
        { enabled: true, position: 'start' },
        { enabled: false, position: 'end' },
      ),
    ).resolves.toBeNull()

    const stored = mockState().projects.find((candidate) => candidate.id === row.id)
    expect(stored?.scheme).toBe('B')
    expect(stored?.baseName).toBe('CLAUDE.md')
    expect(stored?.extraLocalEnabled).toBe(true)
    expect(stored?.extraLocalPosition).toBe('start')
    // The re-sent stored pair is stripped, not churned.
    expect(stored?.extraProjectEnabled).toBe(before.extraProjectEnabled)
    expect(stored?.extraProjectPosition).toBe(before.extraProjectPosition)
    // The §6.9 fields are absent — unchanged.
    expect(stored?.name).toBe(before.name)
    expect(stored?.label).toBe(before.label)
    expect(stored?.description).toBe(before.description)
    expect(stored?.color).toBe(before.color)
    // The immutable half (PRJ-008): the path and the origin stay.
    expect(stored?.path).toBe(before.path)
    expect(stored?.addedVia).toBe(before.addedVia)
    const journal = await changedEntries()
    expect(journal).toHaveLength(journalBefore + 1)
    const entry = journal[0]
    expect(entry?.action).toBe('project.changed')
    expect(entry?.entity_id).toBe(row.id)
    expect(entry?.project_id).toBe(row.id)
    expect(entry?.details).toBeNull()
  })

  it('rejects invalid scheme, file name and extra position literals, changing nothing', async () => {
    // D-204 point 2's vocabulary refusals, the Rust battery's mirror:
    // an off-vocabulary literal refuses the whole save — the row
    // stays verbatim (snapshot first: the store mutates in place)
    // and the journal claims nothing. Case-sensitivity included: a
    // lower-case member is not a member (the schema CHECK's posture).
    const row = activeRow()
    const before = { ...row }
    const journalBefore = (await changedEntries()).length
    const cases: Array<
      [string, string | null, { enabled: boolean; position: string } | null, boolean]
    > = [
      ['scheme', 'D', null, false],
      ['scheme', 'a', null, false],
      ['base_name', 'GEMINI.md', null, false],
      ['base_name', 'agents.md', null, false],
      ['extra_local', null, { enabled: true, position: 'middle' }, false],
      // The OTHER extra's own token (round 1's P3-1): the envelope
      // names the wire field, so a swapped literal in either arm
      // cannot survive here either.
      ['extra_project', null, null, true],
    ]
    for (const [field, scheme, extraLocal, asExtraProject] of cases) {
      await expect(
        commands.projectUpdate(
          row.id,
          null,
          null,
          null,
          null,
          field === 'scheme' ? scheme : null,
          field === 'base_name' ? scheme : null,
          field === 'extra_local' ? extraLocal : null,
          asExtraProject ? { enabled: true, position: 'middle' } : null,
        ),
      ).rejects.toMatchObject({
        code: 'E_VALIDATION',
        params: { field },
      })
    }
    // The pair's own order (PRJ-012's sentence: extra_local before
    // extra_project): a two-fault send with BOTH positions invalid
    // answers about `extra_local`.
    await expect(
      commands.projectUpdate(
        row.id,
        null,
        null,
        null,
        null,
        null,
        null,
        { enabled: true, position: 'middle' },
        { enabled: true, position: 'middle' },
      ),
    ).rejects.toMatchObject({ params: { field: 'extra_local' } })
    // The bounded echo, pinned at the boundary (D-115's cap): an
    // over-long scheme literal echoes truncated with the suffix,
    // never whole.
    await expect(
      commands.projectUpdate(row.id, null, null, null, null, 'A'.repeat(70), null, null, null),
    ).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'scheme',
        value: `${'A'.repeat(64)} (+6 more code points)`,
      },
    })
    const stored = mockState().projects.find((candidate) => candidate.id === row.id)
    expect(stored).toEqual(before)
    expect((await changedEntries()).length).toBe(journalBefore)
  })

  it('answers the new fields after the 6.9 fields, in their own order', async () => {
    // D-204 point 2's ordering mirror: the §6.9 fields answer before
    // the new half, and the new half answers in PRJ-012's own
    // sentence order — scheme before file name before the extras.
    await expect(
      commands.projectUpdate(
        'no-such-row',
        'x'.repeat(101),
        null,
        null,
        null,
        'D',
        null,
        null,
        null,
      ),
    ).rejects.toMatchObject({ params: { field: 'name' } })
    await expect(
      commands.projectUpdate('no-such-row', null, null, null, null, 'D', 'GEMINI.md', null, null),
    ).rejects.toMatchObject({ params: { field: 'scheme' } })
    await expect(
      commands.projectUpdate(
        'no-such-row',
        null,
        null,
        null,
        null,
        null,
        'GEMINI.md',
        { enabled: true, position: 'middle' },
        null,
      ),
    ).rejects.toMatchObject({ params: { field: 'base_name' } })
  })

  it('stores extra files under scheme B (the row, not the scheme window)', async () => {
    // D-204 point 3's store-anyway mirror: the §6.2 columns are NOT
    // NULL on every row and the builder — not the store — is what
    // ignores them under B/C, so a scheme-B row saves an extra-file
    // patch like any other (the modal hides the controls; the wire
    // never refuses them). The row's scheme is stated directly (the
    // mock's row-stated-facts habit), so the act is the extra change
    // alone.
    const row = activeRow()
    row.scheme = 'B'
    const journalBefore = (await changedEntries()).length

    await expect(
      commands.projectUpdate(
        row.id,
        null,
        null,
        null,
        null,
        null,
        null,
        {
          enabled: true,
          position: 'start',
        },
        null,
      ),
    ).resolves.toBeNull()

    const stored = mockState().projects.find((candidate) => candidate.id === row.id)
    expect(stored?.scheme).toBe('B')
    expect(stored?.extraLocalEnabled).toBe(true)
    expect(stored?.extraLocalPosition).toBe('start')
    expect(await changedEntries()).toHaveLength(journalBefore + 1)
  })

  it('the unchanged resend of the write configuration is a no-op', async () => {
    // D-204 point 9's no-op mirror: a re-send of the stored write
    // configuration (the extras as whole enabled-plus-position pairs)
    // writes and journals nothing. The row is snapshotted first: the
    // store mutates rows in place, so comparing the live object with
    // itself can never fail (round 1's P3-5).
    const row = activeRow()
    const before = { ...row }
    const journalBefore = (await changedEntries()).length

    await expect(
      commands.projectUpdate(
        row.id,
        null,
        null,
        null,
        null,
        row.scheme,
        row.baseName,
        { enabled: row.extraLocalEnabled, position: row.extraLocalPosition },
        { enabled: row.extraProjectEnabled, position: row.extraProjectPosition },
      ),
    ).resolves.toBeNull()

    const stored = mockState().projects.find((candidate) => candidate.id === row.id)
    expect(stored).toEqual(before)
    expect((await changedEntries()).length).toBe(journalBefore)
  })

  it('a disabled position change is a real row change (the whole pair)', async () => {
    // D-204 point 9's whole-pair mirror (round 1's P2-2): a
    // position-only change with the flag still off is a real row
    // change — the column moves and the journal records it. A strip
    // that compared `enabled` alone would silently swallow this send.
    const row = activeRow()
    const journalBefore = (await changedEntries()).length

    await expect(
      commands.projectUpdate(
        row.id,
        null,
        null,
        null,
        null,
        null,
        null,
        { enabled: row.extraLocalEnabled, position: 'start' },
        null,
      ),
    ).resolves.toBeNull()

    const stored = mockState().projects.find((candidate) => candidate.id === row.id)
    expect(stored?.extraLocalEnabled).toBe(row.extraLocalEnabled)
    expect(stored?.extraLocalPosition).toBe('start')
    expect(await changedEntries()).toHaveLength(journalBefore + 1)
  })

  it('answers a valid scheme over an unknown id with the id refusal', async () => {
    // The stale-screen ordering mirror (D-201 point 6): the fields
    // validate first, then the row read refuses — a VALID scheme over
    // an unknown id is the id's refusal, never a save.
    const journalBefore = (await changedEntries()).length
    await expect(
      commands.projectUpdate('no-such-row', null, null, null, null, 'B', null, null, null),
    ).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'project_id',
        reason: 'no active project has this id',
      },
    })
    expect((await changedEntries()).length).toBe(journalBefore)
  })
})

describe('settingsGet / settingsSet', () => {
  it('serves the thirteen §6.5 keys in table order with spec defaults', async () => {
    const settings = await commands.settingsGet()
    expect(settings.map((s) => s.key)).toEqual([
      'default.scheme',
      'default.base_name',
      'default.extra_local',
      'default.extra_project',
      'backup.retention',
      'sync.skip_identical',
      'scan.max_depth',
      'scan.exclude_names',
      'history.max_revisions',
      'log.max_entries',
      'log.max_days',
      'ui.language',
      'update.check_on_start',
    ])
    const byKey = new Map(settings.map((s) => [s.key, s.value] as const))
    expect(byKey.get('default.scheme')).toBe('"A"')
    expect(byKey.get('backup.retention')).toBe('10')
    expect(byKey.get('scan.exclude_names')).toBe(
      '["node_modules","vendor",".git","dist","build","target",".venv"]',
    )
    expect(byKey.get('ui.language')).toBe('"en"')
  })

  it('applies a valid change and answers the fresh full map', async () => {
    const fresh = await commands.settingsSet([{ key: 'backup.retention', value: '30' }])
    expect(fresh.find((s) => s.key === 'backup.retention')?.value).toBe('30')
    const reread = await commands.settingsGet()
    expect(reread.find((s) => s.key === 'backup.retention')?.value).toBe('30')
    // Untouched keys keep their values (§7.2: applies without a restart).
    expect(reread.find((s) => s.key === 'default.scheme')?.value).toBe('"A"')
  })

  it('journals one settings.changed entry per changed key', async () => {
    const before = await commands.activityList({ actions: ['settings.changed'], limit: 500 })
    await commands.settingsSet([
      { key: 'backup.retention', value: '20' },
      { key: 'scan.max_depth', value: '8' },
    ])
    const after = await commands.activityList({ actions: ['settings.changed'], limit: 500 })
    const beforeIds = new Set(before.entries.map((e) => e.id))
    const fresh = after.entries.filter((e) => !beforeIds.has(e.id))
    expect(fresh).toHaveLength(2)
    expect(fresh.map((e) => e.entity_id).sort()).toEqual(['backup.retention', 'scan.max_depth'])
    expect(fresh.every((e) => e.result === 'ok' && e.visibility === 'user')).toBe(true)
  })

  it('rejects an unknown key with E_VALIDATION', async () => {
    await expect(commands.settingsSet([{ key: 'nope.mode', value: '1' }])).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { key: 'nope.mode' },
    })
  })

  it('rejects Object.prototype names as setting keys, journaling nothing', async () => {
    const before = mockState().activity.length
    await expect(commands.settingsSet([{ key: 'toString', value: '1' }])).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { key: 'toString' },
    })
    await expect(commands.settingsSet([{ key: 'constructor', value: '1' }])).rejects.toMatchObject({
      code: 'E_VALIDATION',
    })
    expect(mockState().activity).toHaveLength(before)
  })

  it('accepts the range ends (§6.5: backup.retention 1–200)', async () => {
    const fresh = await commands.settingsSet([{ key: 'backup.retention', value: '200' }])
    expect(fresh.find((s) => s.key === 'backup.retention')?.value).toBe('200')
    const freshMin = await commands.settingsSet([{ key: 'backup.retention', value: '1' }])
    expect(freshMin.find((s) => s.key === 'backup.retention')?.value).toBe('1')
  })

  it('rejects a wrong-shape value for another key (default.scheme)', async () => {
    await expect(
      commands.settingsSet([{ key: 'default.scheme', value: '"D"' }]),
    ).rejects.toMatchObject({ code: 'E_VALIDATION', params: { key: 'default.scheme' } })
  })

  it('rejects an extra field inside the extra-hint object, journaling nothing', async () => {
    const before = mockState().activity.length
    await expect(
      commands.settingsSet([
        { key: 'default.extra_local', value: '{"enabled":true,"position":"start","typo":1}' },
      ]),
    ).rejects.toMatchObject({ code: 'E_VALIDATION', params: { key: 'default.extra_local' } })
    expect(mockState().activity).toHaveLength(before)
  })

  it('stores the canonical form, not the caller spelling', async () => {
    const fresh = await commands.settingsSet([
      { key: 'default.extra_local', value: '{"position":"start","enabled":true}' },
    ])
    expect(fresh.find((s) => s.key === 'default.extra_local')?.value).toBe(
      '{"enabled":true,"position":"start"}',
    )
  })

  it('rejects a value outside the key range (§6.5: backup.retention 1–200)', async () => {
    await expect(
      commands.settingsSet([{ key: 'backup.retention', value: '0' }]),
    ).rejects.toMatchObject({ code: 'E_VALIDATION', params: { key: 'backup.retention' } })
  })

  it('rejects a non-JSON value', async () => {
    await expect(
      commands.settingsSet([{ key: 'ui.language', value: 'not json' }]),
    ).rejects.toMatchObject({ code: 'E_VALIDATION' })
  })

  it('applies nothing when any entry of the list is invalid (one transaction)', async () => {
    await expect(
      commands.settingsSet([
        { key: 'backup.retention', value: '30' },
        { key: 'log.max_days', value: '999999' },
      ]),
    ).rejects.toMatchObject({ code: 'E_VALIDATION' })
    const reread = await commands.settingsGet()
    expect(reread.find((s) => s.key === 'backup.retention')?.value).toBe('10')
  })
})

describe('activityList', () => {
  it('hides diagnostic entries unless explicitly requested (LOG-005)', async () => {
    const userPage = await commands.activityList({ include_diagnostic: false })
    expect(userPage.entries.every((e) => e.visibility === 'user')).toBe(true)
    const fullPage = await commands.activityList({ include_diagnostic: true })
    expect(fullPage.total).toBeGreaterThan(userPage.total)
    expect(fullPage.entries.some((e) => e.visibility === 'diagnostic')).toBe(true)
  })

  it('orders newest first', async () => {
    const page = await commands.activityList({ limit: 100 })
    const stamps = page.entries.map((e) => e.ts)
    const sorted = [...stamps].sort().reverse()
    expect(stamps).toEqual(sorted)
  })

  it('pages by limit/offset with a filter-independent total', async () => {
    const full = await commands.activityList({ limit: 500 })
    const firstHalf = await commands.activityList({ limit: 2, offset: 0 })
    const secondHalf = await commands.activityList({ limit: 2, offset: 2 })
    expect(firstHalf.entries).toHaveLength(2)
    expect(firstHalf.total).toBe(full.total)
    expect(firstHalf.entries[0]?.id).toBe(full.entries[0]?.id)
    expect(secondHalf.entries[0]?.id).toBe(full.entries[2]?.id)
    // An offset past the end is an empty page, not an error.
    const past = await commands.activityList({ limit: 2, offset: full.total + 10 })
    expect(past.entries).toEqual([])
    expect(past.total).toBe(full.total)
  })

  it('combines filters with AND', async () => {
    const page = await commands.activityList({
      actions: ['project.added'],
      result: 'ok',
    })
    expect(page.entries.every((e) => e.action === 'project.added' && e.result === 'ok')).toBe(true)
    expect(page.total).toBe(
      (await commands.activityList({ limit: 500 })).entries.filter(
        (e) => e.action === 'project.added' && e.result === 'ok',
      ).length,
    )
  })

  it('filters by project_id exactly', async () => {
    const full = await commands.activityList({ limit: 500 })
    const withProject = full.entries.find((e) => e.project_id !== null)
    expect(withProject).toBeDefined()
    const page = await commands.activityList({
      project_id: withProject?.project_id ?? '',
      limit: 500,
    })
    expect(page.entries.length).toBeGreaterThan(0)
    expect(page.entries.every((e) => e.project_id === withProject?.project_id)).toBe(true)
  })

  it('treats the from_ts/to_ts bounds as inclusive', async () => {
    const full = await commands.activityList({ limit: 500, include_diagnostic: true })
    const stamps = full.entries.map((e) => e.ts).sort()
    const oldest = stamps[0]
    const newest = stamps[stamps.length - 1]
    expect(oldest).toBeDefined()
    expect(newest).toBeDefined()
    const window = await commands.activityList({
      from_ts: oldest,
      to_ts: newest,
      include_diagnostic: true,
      limit: 500,
    })
    expect(window.total).toBe(full.total)
    // One millisecond past the oldest stamp drops it (inclusive bound).
    const plusOneMs = new Date(new Date(oldest ?? 0).getTime() + 1).toISOString()
    const tightened = await commands.activityList({
      from_ts: plusOneMs,
      to_ts: newest,
      include_diagnostic: true,
      limit: 500,
    })
    expect(tightened.total).toBeLessThan(full.total)
  })

  it.each([
    [{ limit: 0 }, { field: 'limit' }],
    [{ limit: 501 }, { field: 'limit' }],
    [{ limit: 2.5 }, { field: 'limit' }],
    [{ offset: -1 }, { field: 'offset' }],
    [{ offset: 0.5 }, { field: 'offset' }],
    [{ from_ts: 'yesterday' }, { field: 'from_ts' }],
    [{ from_ts: '2026-11-20T00:00:00Z' }, { field: 'from_ts' }],
    [{ actions: ['sync.run.aborted'] }, { field: 'actions' }],
    [{ result: 'nope' }, { field: 'result' }],
  ])('rejects %j like the real boundary (E_VALIDATION, %j)', async (query, expected) => {
    await expect(commands.activityList(query)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: expect.objectContaining(expected),
    })
  })

  it('rejects an inverted date range naming from_ts', async () => {
    await expect(
      commands.activityList({
        from_ts: '2026-11-20T10:00:00.000Z',
        to_ts: '2026-11-19T10:00:00.000Z',
      }),
    ).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'from_ts', reason: expect.stringContaining('inverted') },
    })
  })
})

describe('appInfo and the system commands', () => {
  it('serves the About facts (§7.9/UPD-005 literals)', async () => {
    const info = await commands.appInfo()
    expect(info.identifier).toBe('pro.yepbro.eunomiasync')
    expect(info.author).toBe('Den Brooks <denis@yepbro.pro>')
    expect(info.license).toBe('MIT OR Apache-2.0')
    expect(info.repository).toBe('https://github.com/yepbro/EunomiaSync')
    expect(info.version).toBe('0.1.0')
  })

  it('lists db copies newest first', async () => {
    const backups = await commands.dbBackupsList()
    expect(backups.length).toBeGreaterThan(0)
    const stamps = backups.map((b) => b.modified_utc)
    expect(stamps).toEqual([...stamps].sort().reverse())
  })

  it('restores a known copy and rejects an unknown one', async () => {
    const backups = await commands.dbBackupsList()
    await expect(commands.dbRestoreBackup(backups[0]?.name ?? '')).resolves.toBeNull()
    await expect(commands.dbRestoreBackup('nope.sqlite3')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { backup_name: 'nope.sqlite3' },
    })
  })

  it('resolves db_recreate and app_open_data_folder as no-ops', async () => {
    await expect(commands.dbRecreate()).resolves.toBeNull()
    await expect(commands.appOpenDataFolder()).resolves.toBeNull()
  })

  it('dialog_pick_folder answers a path project_add_manual accepts', async () => {
    const picked = await commands.dialogPickFolder()
    expect(typeof picked).toBe('string')
    const added = await commands.projectAddManual(picked ?? '', null)
    expect(added.outcome).toBe('added')
  })
})

describe('exclusionsList / exclusionRemove (the D-10 screen, T-078/D-106)', () => {
  it('exclusionsList serves every ExclusionEntry field of the fixture rows', async () => {
    const rows = await commands.exclusionsList()
    expect(rows).toHaveLength(PROFILE.exclusions)
    const [first] = rows
    expect(first).toBeDefined()
    expect(Object.keys(first ?? {}).sort()).toEqual([
      'created_at',
      'id',
      'path',
      'path_key',
      'reason',
    ])
    expect(first?.path_key).toBe(first?.path)
  })

  it('exclusionsList orders oldest first with the id as the tiebreak inside one millisecond', async () => {
    // Force the tiebreak case: two rows sharing a created_at must come
    // back in id order (the store's stable (created_at, id) read).
    const rows = mockState().exclusions
    expect(rows.length).toBeGreaterThanOrEqual(2)
    const shared = rows[0]?.created_at ?? ''
    if (rows[0]) rows[0].created_at = shared
    if (rows[1]) rows[1].created_at = shared
    const listed = await commands.exclusionsList()
    expect(listed.map((r) => r.created_at)).toEqual([
      shared,
      shared,
      ...rows.slice(2).map((r) => r.created_at),
    ])
    const [a, b] = listed
    expect(a && b && a.id < b.id).toBe(true)
  })

  it('exclusionRemove removes the row and the list reflects it', async () => {
    const before = await commands.exclusionsList()
    const victim = before[0]
    expect(victim).toBeDefined()
    await expect(commands.exclusionRemove(victim?.id ?? '')).resolves.toBeNull()
    const after = await commands.exclusionsList()
    expect(after).toHaveLength(before.length - 1)
    expect(after.some((r) => r.id === victim?.id)).toBe(false)
    // The untouched row survives verbatim.
    expect(after[0]?.id).toBe(before[1]?.id)
  })

  it('exclusionRemove journals exclusion.removed naming the removed row (D-119)', async () => {
    const victim = (await commands.exclusionsList())[0]
    expect(victim).toBeDefined()
    await commands.exclusionRemove(victim?.id ?? '')
    const page = await commands.activityList({ actions: ['exclusion.removed'], limit: 500 })
    const entry = page.entries.find((e) => e.details === JSON.stringify({ path: victim?.path }))
    expect(entry, 'one exclusion.removed entry naming the removed path').toBeDefined()
    expect(entry?.visibility).toBe('user')
    expect(entry?.result).toBe('ok')
    expect(entry?.entity_type).toBe('exclusion')
    expect(entry?.entity_id).toBe(victim?.id)
    expect(entry?.project_id).toBeNull()
  })

  it('exclusionRemove rejects an already-removed id with the typed envelope, changing nothing', async () => {
    const victim = (await commands.exclusionsList())[0]
    expect(victim).toBeDefined()
    await commands.exclusionRemove(victim?.id ?? '')
    const listLength = (await commands.exclusionsList()).length
    const journalLength = mockState().activity.length

    await expect(commands.exclusionRemove(victim?.id ?? '')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'id', value: victim?.id, reason: 'no exclusion with this id' },
    })

    expect(await commands.exclusionsList()).toHaveLength(listLength)
    expect(mockState().activity).toHaveLength(journalLength)
  })
})

describe('markdownPreview', () => {
  it('renders paragraphs and escapes raw HTML (SEC-005)', async () => {
    const preview = await commands.markdownPreview(
      'Line one\n<script>alert(1)</script>\n\nLine two',
    )
    expect(preview.html).not.toContain('<script')
    expect(preview.html).toContain('&lt;script&gt;')
    expect(preview.warnings).toEqual([])
    expect(preview.html).toContain('Line two')
  })

  it('renders fenced code blocks', async () => {
    const preview = await commands.markdownPreview('Intro\n\n```bash\npnpm test\n```\n\nOutro')
    expect(preview.html).toContain('<pre><code>pnpm test</code></pre>')
    expect(preview.warnings).toEqual([])
  })

  it('warns about an unclosed code fence at its opening line (D-15)', async () => {
    const preview = await commands.markdownPreview('First\n\n```ts\nconst x = 1')
    expect(preview.warnings).toEqual([{ kind: 'unclosed_code_fence', line: 3 }])
    // Everything after the opening fence renders as code.
    expect(preview.html).toContain('const x = 1')
  })
})

describe('syncRun (the mock stand-in, T-118/D-144)', () => {
  it('opens the run journal bracket the real backend opens (T-124/D-223)', async () => {
    // The honest mirror: the real `sync_run` writes a
    // `sync.run_started` entry in phase 0's record transaction before
    // anything else of the run exists, so the mock stand-in journals
    // the member first too. The stand-in's other journal rows
    // (per-project + summary) are T-118's shape; the entry order is
    // the mirror's own fact — the Activity window reads newest
    // first, so the start bracket lands at the tail.
    const before = mockState().activity.length
    await commands.syncRun(null)
    const entries = mockState().activity.slice(before)
    expect(entries.map((entry) => entry.action)).toContain('sync.run_started')
    const startEntry = entries.find((entry) => entry.action === 'sync.run_started')
    expect(startEntry?.level).toBe('info')
    expect(startEntry?.result).toBe('ok')
    // The start bracket is the run's FIRST row in append order (the
    // per-project and summary rows follow it).
    expect(entries[0]?.action).toBe('sync.run_started')
  })

  it("clears a written row's §5.7 markings and dirty flag with the sync", async () => {
    // Round-1 P2-3's fix, made falsifiable (round-2's P2-1): the
    // mock's post-sync row update models the backend's after-sync
    // pass — a written file matches its build again, so the markings
    // clear with `dirty` (D-137/D-144). A SKIPPED row is not written:
    // its row state stays untouched (the stand-in's documented shape
    // — the real backend never flags an unavailable project at all).
    const rows = mockState().projects
    const actives = rows.filter((row) => row.state === 'active')
    expect(actives.length).toBeGreaterThanOrEqual(3)
    const modified = actives[0]!
    modified.externalChanged = true
    modified.fileDeleted = false
    const deleted = actives[1]!
    deleted.externalChanged = false
    deleted.fileDeleted = true
    const skipped = actives[2]!
    skipped.available = false
    skipped.unavailableReason = 'missing'
    skipped.externalChanged = true

    const outcome = await commands.syncRun(null)
    expect(outcome.outcome).toBe('completed')

    const served = await commands.projectsList(true)
    const byId = new Map(served.map((entry) => [entry.id, entry]))
    // Written rows: both §5.7 markings and the dirty flag clear, the
    // sync stamps its time.
    for (const row of [modified, deleted]) {
      expect(byId.get(row.id)?.external_changed, `${row.name} was written`).toBe(false)
      expect(byId.get(row.id)?.file_deleted, `${row.name} was written`).toBe(false)
      expect(byId.get(row.id)?.dirty, `${row.name} was written`).toBe(false)
      expect(byId.get(row.id)?.last_synced_at, `${row.name} was written`).not.toBeNull()
    }
    // The skipped row: not written, not stamped — and the mock leaves
    // its row state alone (the documented approximation).
    expect(byId.get(skipped.id)?.last_synced_at).toBeNull()
    expect(byId.get(skipped.id)?.external_changed).toBe(true)
    expect(byId.get(skipped.id)?.file_deleted).toBe(false)
  })
})

describe('syncCancel (the mock stand-in, T-120)', () => {
  it('answers nothing to cancel — the mock run is synchronous', async () => {
    // The stand-in's documented limit, pinned: the mock `syncRun` has
    // no live-run window (it is synchronous), so the slot this
    // command would read is always empty — every call converges on
    // `nothingToCancel`, the arm the real backend gives the
    // stale-click and window-close races (the `scanCancel` twin's
    // idempotence). Nothing was latched, nothing written: the mock
    // store is unchanged.
    const before = structuredClone(mockState().projects)
    const outcome = await commands.syncCancel()
    expect(outcome).toEqual({ outcome: 'nothingToCancel' })
    // Idempotent: the re-send converges.
    await expect(commands.syncCancel()).resolves.toEqual({ outcome: 'nothingToCancel' })
    expect(mockState().projects).toEqual(before)
  })
})

describe('syncStatus / syncLastResult (the mock stand-ins, T-121)', () => {
  it('answers the cold-start facts from the rows before any run', async () => {
    // A fresh reset carries no run memory (the stand-in writes no run
    // rows): the panel's pre-first-sync facts — no run live (the
    // stand-in is synchronous), no success time, no partial attempt —
    // and the dirty count aggregated over the ACTIVE rows' flags the
    // way the backend aggregates its verdicts. The pinned world
    // (seed 21): exactly one of the three active rows is dirty, and
    // DB-014's archived arm means the archived row's flag never
    // counts.
    const rows = mockState().projects
    expect(rows.filter((row) => row.state === 'active' && row.dirty)).toHaveLength(1)

    const status = await commands.syncStatus()
    expect(status).toEqual({
      is_running: false,
      last_success_finished_at: null,
      last_attempt_partial: false,
      dirty_projects: 1,
    })
    await expect(commands.syncLastResult()).resolves.toEqual({ outcome: 'noRuns' })
    // Idempotent: the re-send converges.
    await expect(commands.syncLastResult()).resolves.toEqual({ outcome: 'noRuns' })

    // The count follows the rows: flagging a clean active row moves
    // the pill's number, and the archived row's flag never does.
    const archived = rows.find((row) => row.state === 'archived')
    expect(archived).toBeDefined()
    archived!.dirty = true
    const cleanActive = rows.find((row) => row.state === 'active' && !row.dirty)
    expect(cleanActive).toBeDefined()
    cleanActive!.dirty = true
    expect((await commands.syncStatus()).dirty_projects).toBe(2)
  })

  it('serves the last run from the one-run memory, MAIN-027 mirrored', async () => {
    // The card's DoD over the mock's own facts: a SUCCESS run moves
    // the last-success time and is answerable as the last result; a
    // PARTIAL attempt after it (an unavailable project skips, the
    // others are written) moves the success time NOTHING and flips
    // the partial fact, and the summary carries the skipped row's
    // reason (OUT-009/MAIN-031). The memory is one run deep: a later
    // run replaces the answered summary wholesale.
    const actives = mockState().projects.filter((row) => row.state === 'active')
    expect(actives.length).toBeGreaterThanOrEqual(3)

    const success = await commands.syncRun(null)
    expect(success.outcome).toBe('completed')
    if (success.outcome !== 'completed') return
    expect(success.summary.verdict).toBe('success')

    let status = await commands.syncStatus()
    expect(status.is_running).toBe(false)
    expect(status.last_success_finished_at).toBe(success.summary.finished_at)
    expect(status.last_attempt_partial).toBe(false)
    let answered = await commands.syncLastResult()
    expect(answered.outcome).toBe('completed')
    if (answered.outcome !== 'completed') return
    expect(answered.summary).toEqual(success.summary)
    // The stand-in has no abort paths: the run-level code is the
    // honest "no code" (the real backend's finished runs answer the
    // same through the seam).
    expect(answered.error_code).toBeNull()

    // The partial attempt: one active row goes unavailable → skipped,
    // the others written → `partial` (the stand-in's own rule).
    const skipped = actives[2]!
    skipped.available = false
    skipped.unavailableReason = 'missing'
    // The sentinel (round-1's P2 fix): the two mock runs' stamps share
    // the millisecond this side of determinism, so the equality
    // assertion below cannot discriminate a mock that wrongly moves
    // the time on a `partial` run — the sentinel makes it a fact: if
    // the partial run moved `lastSyncSuccessAt`, the member answers
    // the partial run's `now` and the pin fails.
    mockState().lastSyncSuccessAt = '2026-01-01T00:00:00.000Z'
    const partial = await commands.syncRun(null)
    expect(partial.outcome).toBe('completed')
    if (partial.outcome !== 'completed') return
    expect(partial.summary.verdict).toBe('partial')

    status = await commands.syncStatus()
    // MAIN-027 mirrored: the partial attempt moves the success time
    // nothing — the member keeps the sentinel the success run left.
    expect(status.last_success_finished_at).toBe('2026-01-01T00:00:00.000Z')
    expect(status.last_attempt_partial).toBe(true)
    // OUT-020 mirrored: the skipped project keeps its previous sync
    // state — the first run cleared its flag, so the pill reads zero
    // (the backend's DB-021 derivation would count a NEVER-synced
    // skipped project; the stand-in's row-flag approximation is the
    // documented boundary, D-217 point 6).
    expect(status.dirty_projects).toBe(0)

    answered = await commands.syncLastResult()
    expect(answered.outcome).toBe('completed')
    if (answered.outcome !== 'completed') return
    expect(answered.summary).toEqual(partial.summary)
    expect(answered.summary.run_id).not.toBe(success.summary.run_id)
    // The reason rides the skipped row (OUT-009/MAIN-031).
    const row = answered.summary.projects.find((p) => p.project_id === skipped.id)
    expect(row?.status).toBe('skipped')
    expect(row?.error_code).toBe('E_PROJECT_UNAVAILABLE')
    expect(row?.error_message).not.toBeNull()
  })
})

describe('syncConfirmExit (the mock stand-in, T-123)', () => {
  it('accepts the confirm and converges on the re-send', async () => {
    // The stand-in's documented limits, pinned: the mock has no
    // window machinery (nothing intercepts a close, so no ask ever
    // fires on the mock bus) and no process to end — the EXIT itself
    // is not model-able in a browser mock. What the twin serves is
    // the command's CONTRACT: the call is accepted (the latch
    // records it — the "the confirm landed" fact the dialog's future
    // tests read), the converging re-send answers the same `null`
    // the real double-click arm gives, and nothing else in the mock
    // store changes (the wait has no mock meaning: the `syncRun`
    // stand-in is synchronous, no write is ever in flight).
    expect(mockState().exitConfirmed).toBe(false)
    const before = structuredClone(mockState().projects)

    await expect(commands.syncConfirmExit()).resolves.toBeNull()

    expect(mockState().exitConfirmed).toBe(true)
    // Idempotent: the re-send converges.
    await expect(commands.syncConfirmExit()).resolves.toBeNull()
    expect(mockState().exitConfirmed).toBe(true)
    expect(mockState().projects).toEqual(before)
  })
})

describe('scanStart / scanCancel', () => {
  /** The fixture world's scan root: every project lives under it. */
  const ROOT = '/home/dev/src'

  it('classifies the store over the root and answers the candidate set', async () => {
    // The mock's registry classes: the PROFILE world holds 3 active
    // (known), 1 archived, 2 excluded paths under the root, and the
    // synthetic discovery keeps "New" non-empty; the first §6.5
    // default exclude name is the one skipped folder.
    const outcome = await commands.scanStart(ROOT)

    expect(outcome.outcome).toBe('completed')
    if (outcome.outcome !== 'completed') return
    const classes = new Set(outcome.repositories.map((repo) => repo.class))
    expect(outcome.repositories.filter((repo) => repo.class === 'known')).toHaveLength(3)
    expect(outcome.repositories.filter((repo) => repo.class === 'archived')).toHaveLength(1)
    expect(outcome.repositories.filter((repo) => repo.class === 'excluded')).toHaveLength(2)
    expect(outcome.repositories.filter((repo) => repo.class === 'new')).toHaveLength(1)
    expect(outcome.found).toBe(7)
    expect(outcome.checked).toBeGreaterThan(outcome.found)
    expect(outcome.skipped).toEqual([
      { path: `${ROOT}/node_modules`, reason: 'excluded', os_code: null },
    ])
    // The store is untouched either way: nothing was added (ERR-002's
    // twin — adding is `scanCommit`'s job, see its suite below).
    expect(mockState().projects).toHaveLength(4)
    expect(classes.size).toBe(4)
  })

  it('emits scan://progress while it walks, climbing to the terminal counts', async () => {
    const seen: Array<{ checked: number; found: number; current_path: string }> = []
    const stop = mockEvents.scanProgress.on((payload) => seen.push(payload))
    try {
      const outcome = await commands.scanStart(ROOT)
      if (outcome.outcome !== 'completed') throw new Error('the uncanceled walk must complete')
      expect(seen.length).toBeGreaterThan(1)
      for (const pair of seen.flatMap((payload, i) => (i > 0 ? [[seen[i - 1]!, payload]] : []))) {
        expect(pair[0]!.checked).toBeLessThanOrEqual(pair[1]!.checked)
        expect(pair[0]!.found).toBeLessThanOrEqual(pair[1]!.found)
      }
      for (const payload of seen) {
        expect(payload.current_path.startsWith(ROOT)).toBe(true)
        expect(payload.checked).toBeLessThanOrEqual(outcome.checked)
        expect(payload.found).toBeLessThanOrEqual(outcome.found)
      }
    } finally {
      stop()
    }
  })

  it('a cancel mid-walk answers the payload-less cancelled outcome, adding nothing', async () => {
    const pending = commands.scanStart(ROOT)
    await commands.scanCancel()
    const outcome = await pending

    expect(outcome).toEqual({ outcome: 'cancelled' })
    expect(mockState().projects).toHaveLength(4)
    expect(mockState().scanning).toBe(false)
  })

  it('a second start while one is live rejects E_SYNC_RUNNING', async () => {
    const pending = commands.scanStart(ROOT)
    await expect(commands.scanStart(ROOT)).rejects.toMatchObject({
      code: 'E_SYNC_RUNNING',
    })
    const outcome = await pending
    expect(outcome.outcome).toBe('completed')
    expect(mockState().scanning).toBe(false)
  })

  it('a cancel with nothing live is the idempotent no-op', async () => {
    await expect(commands.scanCancel()).resolves.toBeNull()
  })

  it('rejects an empty path with the E_VALIDATION field envelope', async () => {
    await expect(commands.scanStart('   ')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'path' },
    })
  })
})

describe('scanCommit', () => {
  /** The fixture world's scan root and the synthetic discovery its walk reports. */
  const ROOT = '/home/dev/src'
  const DISCOVERED = `${ROOT}/discovered-repo`

  /** One completed scan over the fixture world — the commit's precondition. */
  async function scanWorld(): Promise<void> {
    const outcome = await commands.scanStart(ROOT)
    if (outcome.outcome !== 'completed') throw new Error('the uncanceled walk must complete')
  }

  it('adds the discovered repository with the manual-add rules and journals it', async () => {
    // T-077/D-179, mock side: the one synthetic discovery goes through the
    // SAME row-building step as `projectAddManual` (name from the folder,
    // DB-042 label, D-29 color, appended `sort_order`, settings copied from
    // `default.*`) with `added_via = 'scan'`, and journals one
    // `project.added` entry naming the row.
    await scanWorld()
    const state = mockState()
    const before = state.projects.length
    const lastSortOrder = Math.max(...state.projects.map((row) => row.sortOrder))

    const result = await commands.scanCommit([DISCOVERED])

    expect(result.entries).toHaveLength(1)
    const entry = result.entries[0]
    expect(entry?.outcome).toBe('added')
    expect(entry?.path).toBe(DISCOVERED)
    expect(entry?.existing).toBeNull()
    expect(entry?.project).toMatchObject({
      path: DISCOVERED,
      name: 'discovered-repo',
      label: 'DISCOV',
      added_via: 'scan',
      sort_order: lastSortOrder + 1,
      write_access_warning: null,
    })
    expect(PALETTE).toContain(entry?.project?.color)
    expect(state.projects).toHaveLength(before + 1)
    expect(state.activity[state.activity.length - 1]).toMatchObject({
      action: 'project.added',
      level: 'info',
      visibility: 'user',
      result: 'ok',
      entity_type: 'project',
      project_id: entry?.project?.id,
    })
  })

  it('rejects a path no completed scan reported, before anything is added', async () => {
    // The discovered-set rule (SEC-003, D-179): a path under the scanned
    // root that the walk did not report is the typed `E_VALIDATION` of the
    // `path` field echoing the caller's input, and nothing is added — with
    // a reported path in the same selection, either. Before any completed
    // scan nothing is reported at all.
    await scanWorld()
    const before = mockState().projects.length
    const never = `${ROOT}/never-reported`

    await expect(commands.scanCommit([DISCOVERED, never])).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'path', value: never },
    })
    expect(mockState().projects).toHaveLength(before)
    await expect(commands.scanCommit(['   '])).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'path' },
    })

    resetMockState(PROFILE)
    await expect(commands.scanCommit([DISCOVERED])).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'path' },
    })
  })

  it('answers typed outcomes, lifts an exclusion into an added row, and adds the rest', async () => {
    // The typed non-add outcomes mirror the real command: an active row is
    // `already_added` naming that row, an archived one `archived` with the
    // restore offer's id; an excluded path commits (D-196) — the
    // exclusion row goes, the lift is journaled
    // `exclusion.removed` with the created row as its `project_id`, and
    // the path answers `added` like any other — and the discovery in the
    // same selection is still added. Re-sending the selection converges
    // (the lifted path is `already_added` too, nothing new written).
    await scanWorld()
    const state = mockState()
    const known = state.projects.find((row) => row.state === 'active')
    const archived = state.projects.find((row) => row.state === 'archived')
    const excluded = state.exclusions[0]
    if (!known || !archived || !excluded) throw new Error('the fixture world must hold all three')
    const before = state.projects.length
    const exclusionsBefore = state.exclusions.length
    const journalBefore = state.activity.length

    const first = await commands.scanCommit([archived.path, excluded.path, DISCOVERED, known.path])
    const byPath = new Map(first.entries.map((entry) => [entry.path, entry]))

    expect(first.entries.map((entry) => entry.path)).toEqual([
      archived.path,
      excluded.path,
      DISCOVERED,
      known.path,
    ])
    expect(byPath.get(archived.path)).toMatchObject({
      outcome: 'archived',
      project: null,
      existing: { id: archived.id, state: 'archived', path: archived.path },
    })
    expect(byPath.get(excluded.path)?.outcome).toBe('added')
    expect(byPath.get(excluded.path)?.project?.path).toBe(excluded.path)
    expect(byPath.get(DISCOVERED)?.outcome).toBe('added')
    expect(byPath.get(known.path)).toMatchObject({
      outcome: 'already_added',
      project: null,
      existing: { id: known.id, state: 'active' },
    })
    expect(state.projects).toHaveLength(before + 2)
    expect(state.exclusions).toHaveLength(exclusionsBefore - 1)
    expect(state.activity).toHaveLength(journalBefore + 3)
    expect(state.activity.slice(-3)).toEqual([
      expect.objectContaining({
        action: 'project.added',
        project_id: byPath.get(excluded.path)?.project?.id,
      }),
      expect.objectContaining({
        action: 'exclusion.removed',
        entity_type: 'exclusion',
        entity_id: excluded.id,
        project_id: byPath.get(excluded.path)?.project?.id,
      }),
      expect.objectContaining({
        action: 'project.added',
        entity_type: 'project',
      }),
    ])

    const second = await commands.scanCommit([archived.path, excluded.path, DISCOVERED, known.path])
    expect(second.entries.map((entry) => entry.outcome)).toEqual([
      'archived',
      'already_added',
      'already_added',
      'already_added',
    ])
    expect(state.projects).toHaveLength(before + 2)
    expect(state.exclusions).toHaveLength(exclusionsBefore - 1)
    expect(state.activity).toHaveLength(journalBefore + 3)
  })

  it('collapses duplicate spellings and answers an empty selection with no entries', async () => {
    // The same repository sent twice (once with a trailing separator, which
    // keys alike) is one entry and one row; the empty selection is the
    // typed no-op.
    await scanWorld()
    const state = mockState()
    const before = state.projects.length

    const result = await commands.scanCommit([DISCOVERED, `${DISCOVERED}/`, DISCOVERED])

    expect(result.entries).toHaveLength(1)
    expect(state.projects.filter((row) => row.path === DISCOVERED)).toHaveLength(1)
    expect(state.projects).toHaveLength(before + 1)
    await expect(commands.scanCommit([])).resolves.toEqual({ entries: [], probe_failed: false })
    expect(state.projects).toHaveLength(before + 1)
  })

  it('a cancelled scan publishes nothing', async () => {
    // ERR-002: only a completed walk publishes its findings.
    const pending = commands.scanStart(ROOT)
    await commands.scanCancel()
    expect((await pending).outcome).toBe('cancelled')

    await expect(commands.scanCommit([DISCOVERED])).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'path' },
    })
  })
})

// ---------------------------------------------------------------------------
// The four instruction commands (T-328, §7.10 Library module, D-188)
// ---------------------------------------------------------------------------

describe('instructionCreate / instructionUpdate / instructionHide / instructionRestore (T-328)', () => {
  /** The ids of the seeded two-level world below. */
  const H_A = 'h-a'
  const I_ROOT = 'i-root'
  const H_B = 'h-b'
  const I_X = 'i-x'
  const H_C = 'h-c'

  /**
   * A small two-level library world (the fixture profile's single
   * heading sits at the table tail, which cannot exercise the section
   * rule — so the block seeds its own rows, the state-editing pattern
   * the watch-flag tests use): `## A` (1), a root instruction (2),
   * `### B` (3), X (4), `## C` (5).
   */
  function seedTwoLevelLibrary(): void {
    // The rows follow the store's union shape (dates included — the
    // seeded row's fields are the shape every reader maps).
    const stamp = (position: number): string =>
      new Date(Date.UTC(2026, 10, 24, 12, 0, 0) + position * 1000).toISOString()
    mockState().library = [
      {
        kind: 'heading',
        id: H_A,
        position: 1,
        headingLevel: 2,
        title: 'A section',
        state: 'active',
        createdAt: stamp(1),
        updatedAt: stamp(1),
      },
      {
        kind: 'instruction',
        id: I_ROOT,
        position: 2,
        title: 'Root rule',
        state: 'active',
        variants: [
          {
            id: 'v-root',
            name: 'Main',
            content: 'Root text.',
            isMain: true,
            state: 'active',
            createdAt: stamp(2),
            updatedAt: stamp(2),
            deletedAt: null,
          },
        ],
        createdAt: stamp(2),
        updatedAt: stamp(2),
      },
      {
        kind: 'heading',
        id: H_B,
        position: 3,
        headingLevel: 3,
        title: 'B subsection',
        state: 'active',
        createdAt: stamp(3),
        updatedAt: stamp(3),
      },
      {
        kind: 'instruction',
        id: I_X,
        position: 4,
        title: 'X rule',
        state: 'active',
        variants: [
          {
            id: 'v-x',
            name: 'Main',
            content: 'X text.',
            isMain: true,
            state: 'active',
            createdAt: stamp(4),
            updatedAt: stamp(4),
            deletedAt: null,
          },
        ],
        createdAt: stamp(4),
        updatedAt: stamp(4),
      },
      {
        kind: 'heading',
        id: H_C,
        position: 5,
        headingLevel: 2,
        title: 'C section',
        state: 'active',
        createdAt: stamp(5),
        updatedAt: stamp(5),
      },
    ]
  }

  /** The journal's current length. */
  const journalSize = (): number => mockState().activity.length

  it('creates the pair with the Main variant, places it, and journals once', async () => {
    // T-084's main flow (LIB-002): one instruction row with its Main
    // variant, the name stored trimmed, the placement at the end of
    // the picked section, and ONE `instruction.created` entry.
    seedTwoLevelLibrary()
    const state = mockState()
    const before = journalSize()

    const created = await commands.instructionCreate('  New rule  ', 'Be concise.', H_B)

    const row = state.library.find((item) => item.id === created.instruction_id)
    expect(row).toMatchObject({
      kind: 'instruction',
      title: 'New rule',
      state: 'active',
      position: 5,
    })
    // The end of `### B`'s range is `## C` (position 5): the new row
    // takes position 5 and C shifts down — the dense renumber; X (the
    // range's last item) keeps its position 4.
    expect(state.library.find((item) => item.id === I_X)?.position).toBe(4)
    expect(state.library.find((item) => item.id === H_C)?.position).toBe(6)
    // The union narrows by kind: the created row is the instruction.
    // `toMatchObject` (the landing's mechanical consequence of
    // MockVariantRow's T-329 extension): the row now also carries the
    // variant-command columns with wall-clock dates, so exhaustive
    // equality cannot name them — the four pinned facts are unchanged.
    expect(row?.kind === 'instruction' ? row.variants : undefined).toMatchObject([
      { id: created.variant_id, name: 'Main', content: 'Be concise.', isMain: true },
    ])
    expect(created.position).toBe(5)
    expect(journalSize()).toBe(before + 1)
    expect(state.activity[state.activity.length - 1]).toMatchObject({
      action: 'instruction.created',
      entity_type: 'instruction',
      entity_id: created.instruction_id,
    })
  })

  it('places a root create before the first heading (ALG-021, D-188)', async () => {
    seedTwoLevelLibrary()

    const created = await commands.instructionCreate('Root twin', 'Text.', null)

    // The root prefix ends at the FIRST heading (position 1), so a
    // root create lands at position 1 — the only place with no
    // heading above it.
    expect(created.position).toBe(1)
    expect(mockState().library[0]?.id).toBe(created.instruction_id)
    expect(mockState().library.find((item) => item.id === H_A)?.position).toBe(2)
  })

  it('creates into a level-2 section past its nested subsection (the discriminating case)', async () => {
    // The round-1 review's TA-P2-2, mock side: the end of `## A`'s
    // range is `## C` (the next heading of level <= 2) — NOT `### B`
    // (the first following heading of ANY level) and not the table
    // end. Both wrong readings answer a different position here.
    seedTwoLevelLibrary()

    const created = await commands.instructionCreate('New in A', 'Text.', H_A)

    expect(created.position).toBe(5)
    expect(mockState().library.find((item) => item.id === H_B)?.position).toBe(3)
    expect(mockState().library.find((item) => item.id === I_X)?.position).toBe(4)
    expect(mockState().library.find((item) => item.id === H_C)?.position).toBe(6)
  })

  it('rejects a NOCASE-twin visible name with the typed envelope, writing nothing', async () => {
    // §6.9's cross-row clause: unique among VISIBLE instructions,
    // case-insensitive — the refusal names the field and echoes the
    // caller's own (validated) name, and nothing is written or
    // journaled. The positive half rides along: a HIDDEN twin does
    // not block the create, and SQLite's NOCASE fold is ASCII-only,
    // so a non-ASCII "twin" never blocked in the first place.
    seedTwoLevelLibrary()
    const state = mockState()
    const before = journalSize()
    const rows = state.library.length

    await expect(
      commands.instructionCreate('  ROOT RULE  ', 'Twin text.', null),
    ).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'name', value: 'ROOT RULE' },
    })

    expect(state.library).toHaveLength(rows)
    expect(journalSize()).toBe(before)

    // A hidden twin does not block (§6.9: "among visible"). The root
    // placement applies: the create lands before the first heading
    // (position 1).
    await commands.instructionHide(I_ROOT)
    await expect(
      commands.instructionCreate('root rule', 'The visible twin.', null),
    ).resolves.toMatchObject({ position: 1 })
    // The ASCII-only fold's own pin (SQLite's NOCASE folds A–Z only):
    // "MÜLLER" is NOT a NOCASE twin of an existing "Müller" — the Ü
    // does not fold, so the create succeeds (a Unicode-fold mutant
    // would refuse it).
    await commands.instructionCreate('Müller', 'Text.', null)
    await expect(commands.instructionCreate('MÜLLER', 'Text.', null)).resolves.toMatchObject({
      position: 3,
    })
  })

  it('answers a double fault about the name (the fixed validation order)', async () => {
    // The real command's order, pinned: name first, then text — a
    // request violating both fields answers about the NAME.
    seedTwoLevelLibrary()

    await expect(commands.instructionCreate('', 'a\0b', null)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'name' },
    })
    // And the section precedes the duplicate at the store boundary
    // (the real command computes `section_target` before `create_item`
    // meets the twin): a twin name AND a non-heading section answer
    // about the section.
    await expect(commands.instructionCreate('Root rule', 'Text.', I_X)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'section' },
    })
  })

  it('rejects a reserved marker line with E_CONTENT_RESERVED (DB-016)', async () => {
    seedTwoLevelLibrary()
    const state = mockState()
    const before = journalSize()

    await expect(
      commands.instructionCreate('Valid name', 'Before.\n<!-- EUNOMIASYNC:END -->\nAfter.', null),
    ).rejects.toMatchObject({
      code: 'E_CONTENT_RESERVED',
      params: { field: 'text', marker: '<!-- EUNOMIASYNC:END -->', line: '2' },
    })

    expect(state.library).toHaveLength(5)
    expect(journalSize()).toBe(before)
  })

  it('numbers the marker line over the built form (ALG-002 leading edge)', async () => {
    // The real validator scans the ALG-002-normalized text — the
    // leading empty lines drop, so a marker under them is line 1 of
    // the built form, and space/tab padding matches (Unicode
    // whitespace padding does not — `classify_line`'s own rule).
    seedTwoLevelLibrary()

    await expect(
      commands.instructionCreate(
        'Valid name',
        '\n\n \t<!-- EUNOMIASYNC:BEGIN -->\t \nAfter.',
        null,
      ),
    ).rejects.toMatchObject({
      code: 'E_CONTENT_RESERVED',
      params: { line: '1' },
    })
  })

  it('rejects a marker a leading BOM-after-blank-line was hiding (the fixed-point edge)', async () => {
    // `drop_leading_edge`'s own rule, pinned: a BOM that becomes
    // LEADING only after a blank line drops is still leading — the
    // round-2 review's P3-2, the one shape where a single-pass strip
    // misses the marker the app rejects. The BOM here sits after a
    // blank line, so the built form's line 1 is the bare marker.
    seedTwoLevelLibrary()

    await expect(
      commands.instructionCreate('Valid name', '\n\uFEFF<!-- EUNOMIASYNC:END -->\nAfter.', null),
    ).rejects.toMatchObject({
      code: 'E_CONTENT_RESERVED',
      params: { line: '1' },
    })
  })

  it('stores the create text in the save-folded form (CRLF becomes LF)', async () => {
    // §6.9's save-time fold, the store's `fold_crlf` twin: CRLF
    // becomes LF on save (a lone CR stays). The stored row a later
    // read serves carries the folded form.
    seedTwoLevelLibrary()

    const created = await commands.instructionCreate('Folded rule', 'Line one.\r\nLine two.', null)

    const created_row = mockState().library.find((item) => item.id === created.instruction_id)
    expect(created_row?.kind === 'instruction' ? created_row.variants[0] : undefined).toMatchObject(
      { content: 'Line one.\nLine two.' },
    )
  })

  it('rejects the §6.9 name and text violations, and a section that is not a heading', async () => {
    seedTwoLevelLibrary()
    const state = mockState()
    const before = journalSize()

    await expect(commands.instructionCreate('   ', 'Text.', null)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'name' },
    })
    await expect(commands.instructionCreate('a\nb', 'Text.', null)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'name' },
    })
    await expect(
      commands.instructionCreate('Name', `x${'y'.repeat(262_144)}`, null),
    ).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'text' },
    })
    // MAIN-015's identity rule: an instruction id is not a section.
    await expect(commands.instructionCreate('Name', 'Text.', I_X)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'section' },
    })
    await expect(commands.instructionCreate('Name', 'Text.', 'no-such-row')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'section' },
    })

    expect(state.library).toHaveLength(5)
    expect(journalSize()).toBe(before)
  })

  it('renames and journals, and answers a same-name rename as the verified no-op', async () => {
    // T-085's DoD (D-188): the same-name save writes and journals
    // nothing; a real rename journals `instruction.changed`.
    seedTwoLevelLibrary()
    const state = mockState()
    const before = journalSize()

    await commands.instructionUpdate(I_X, '  X rule  ')

    // The no-op: the trimmed name equals the stored title.
    expect(state.library.find((item) => item.id === I_X)?.title).toBe('X rule')
    expect(journalSize()).toBe(before)

    await commands.instructionUpdate(I_X, 'Renamed rule')
    expect(state.library.find((item) => item.id === I_X)?.title).toBe('Renamed rule')
    expect(journalSize()).toBe(before + 1)
    expect(state.activity[state.activity.length - 1]).toMatchObject({
      action: 'instruction.changed',
      entity_id: I_X,
    })
  })

  it('refuses a rename onto a visible twin, a heading id, and a stale id', async () => {
    seedTwoLevelLibrary()
    const state = mockState()
    const before = journalSize()

    await expect(commands.instructionUpdate(I_X, 'root rule')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'name', value: 'root rule' },
    })
    await expect(commands.instructionUpdate(H_A, 'New title')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'id' },
    })
    await expect(commands.instructionUpdate('no-such-row', 'New title')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'id' },
    })

    expect(state.library.find((item) => item.id === I_X)?.title).toBe('X rule')
    expect(journalSize()).toBe(before)
  })

  it('hides keeping the variants, and restores — one journal entry per real transition', async () => {
    // T-086's lifecycle (D-11/DB-013: the variants stay; the mock has
    // no assignment rows to keep — the documented limit), with the
    // D-188 idempotence: a repeated hide or restore writes and
    // journals nothing.
    seedTwoLevelLibrary()
    const state = mockState()
    const before = journalSize()

    await commands.instructionHide(I_X)
    expect(state.library.find((item) => item.id === I_X)).toMatchObject({
      state: 'hidden',
      variants: [{ id: 'v-x', name: 'Main', content: 'X text.', isMain: true }],
    })
    expect(journalSize()).toBe(before + 1)
    expect(state.activity[state.activity.length - 1]).toMatchObject({
      action: 'instruction.hidden',
      entity_id: I_X,
    })

    await commands.instructionHide(I_X)
    expect(journalSize()).toBe(before + 1)

    await commands.instructionRestore(I_X)
    expect(state.library.find((item) => item.id === I_X)?.state).toBe('active')
    expect(journalSize()).toBe(before + 2)
    expect(state.activity[state.activity.length - 1]).toMatchObject({
      action: 'instruction.restored',
      entity_id: I_X,
    })

    await commands.instructionRestore(I_X)
    expect(journalSize()).toBe(before + 2)
  })

  it('refuses a restore into a visible NOCASE twin, class-only', async () => {
    // §6.9's clause surfacing at the restore (D-076): the envelope
    // carries the field and the reason, never the stored title
    // (SEC-008).
    seedTwoLevelLibrary()
    const state = mockState()

    await commands.instructionHide(I_ROOT)
    // A visible twin of the hidden name appears while it is hidden.
    await commands.instructionCreate('ROOT RULE', 'Twin text.', null)

    await expect(commands.instructionRestore(I_ROOT)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'name' },
    })
    expect(state.library.find((item) => item.id === I_ROOT)?.state).toBe('hidden')
  })

  it('refuses a hide and a restore on a heading id and a stale id', async () => {
    seedTwoLevelLibrary()
    const state = mockState()
    const before = journalSize()

    await expect(commands.instructionHide(H_B)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'id' },
    })
    await expect(commands.instructionRestore('no-such-row')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'id' },
    })

    expect(state.library.find((item) => item.id === H_B)?.state).toBe('active')
    expect(journalSize()).toBe(before)
  })
})

// ---------------------------------------------------------------------------
// The four structure commands (T-330): the mock twins of the Rust battery
// over the same union-shaped library rows.
// ---------------------------------------------------------------------------

describe('headingCreate / headingUpdate / headingDelete / itemsMove (T-330)', () => {
  const H_A = 'structure-h-a'
  const I_A = 'structure-i-a'
  const H_B = 'structure-h-b'
  const I_B = 'structure-i-b'
  const H_C = 'structure-h-c'
  const I_C = 'structure-i-c'

  function seedStructureLibrary(): void {
    const stamp = (position: number): string =>
      new Date(Date.UTC(2026, 10, 24, 12, 0, 0) + position * 1000).toISOString()
    const instruction = (id: string, position: number, hidden = false): MockLibraryRow => ({
      kind: 'instruction',
      id,
      position,
      title: id,
      state: hidden ? 'hidden' : 'active',
      variants: [
        {
          id: `${id}-main`,
          name: 'Main',
          content: `${id} text`,
          isMain: true,
          state: 'active',
          createdAt: stamp(position),
          updatedAt: stamp(position),
          deletedAt: null,
        },
      ],
      createdAt: stamp(position),
      updatedAt: stamp(position),
    })
    mockState().library = [
      {
        kind: 'heading',
        id: H_A,
        position: 1,
        headingLevel: 2,
        title: 'A',
        state: 'active',
        createdAt: stamp(1),
        updatedAt: stamp(1),
      },
      instruction(I_A, 2),
      {
        kind: 'heading',
        id: H_B,
        position: 3,
        headingLevel: 3,
        title: 'B',
        state: 'active',
        createdAt: stamp(3),
        updatedAt: stamp(3),
      },
      instruction(I_B, 4, true),
      {
        kind: 'heading',
        id: H_C,
        position: 5,
        headingLevel: 2,
        title: 'C',
        state: 'active',
        createdAt: stamp(5),
        updatedAt: stamp(5),
      },
      instruction(I_C, 6),
    ]
  }

  const ids = (): string[] =>
    [...mockState().library]
      .sort((left, right) => left.position - right.position)
      .map((row) => row.id)
  const heading = (id: string) => {
    const row = mockState().library.find((candidate) => candidate.id === id)
    return row?.kind === 'heading' ? row : undefined
  }

  it('validates create input and places root/child headings with one journal entry each', async () => {
    seedStructureLibrary()
    const state = mockState()
    const beforeRows = ids()
    const beforeJournal = state.activity.length
    for (const name of ['', '   ', 'a\nb', 'a\0b', 'x'.repeat(201)]) {
      await expect(commands.headingCreate(name, 2, null)).rejects.toMatchObject({
        code: 'E_VALIDATION',
        params: { field: 'name' },
      })
    }
    for (const level of [1, 7, -1, 256, 300]) {
      await expect(commands.headingCreate('Valid', level, null)).rejects.toMatchObject({
        code: 'E_VALIDATION',
        params: { field: 'level' },
      })
    }
    await expect(commands.headingCreate('Valid', 3, I_A)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'parent' },
    })
    expect(ids()).toEqual(beforeRows)
    expect(state.activity).toHaveLength(beforeJournal)

    const child = await commands.headingCreate('  Child  ', 4, H_B)
    const root = await commands.headingCreate('Root', 2, null)
    expect(child).toMatchObject({ title: 'Child', level: 4, position: 5 })
    expect(root).toMatchObject({ title: 'Root', level: 2, position: 8 })
    expect(ids()).toEqual([H_A, I_A, H_B, I_B, child.id, H_C, I_C, root.id])
    expect(state.activity.slice(beforeJournal).map((entry) => entry.action)).toEqual([
      'heading.created',
      'heading.created',
    ])
  })

  it('renames, changes one level and shifts a branch, with verified no-ops', async () => {
    seedStructureLibrary()
    const state = mockState()
    const beforeJournal = state.activity.length
    await expect(
      commands.headingUpdate(H_A, { mode: 'rename', value: '  Renamed  ' }),
    ).resolves.toMatchObject({ title: 'Renamed' })
    await expect(
      commands.headingUpdate(H_A, { mode: 'rename', value: 'Renamed' }),
    ).resolves.toMatchObject({ title: 'Renamed' })
    expect(state.activity).toHaveLength(beforeJournal + 1)

    await commands.headingUpdate(H_B, { mode: 'change_level', value: 5 })
    await commands.headingUpdate(H_B, { mode: 'change_level', value: 5 })
    expect(state.activity).toHaveLength(beforeJournal + 2)
    await commands.headingUpdate(H_B, { mode: 'shift_branch', value: 1 })
    expect(heading(H_B)?.headingLevel).toBe(6)
    expect(state.activity).toHaveLength(beforeJournal + 3)
    expect(state.activity[state.activity.length - 1]).toMatchObject({
      action: 'heading.changed',
      entity_type: 'heading',
      entity_id: H_B,
      details: JSON.stringify({ changed: 1 }),
    })
  })

  it('validates every rename before writing or journaling', async () => {
    seedStructureLibrary()
    const state = mockState()
    const beforeRows = JSON.stringify(state.library)
    const beforeJournal = state.activity.length
    for (const name of ['', '   ', 'a\nb', 'a\0b', 'x'.repeat(201)]) {
      await expect(
        commands.headingUpdate(H_A, { mode: 'rename', value: name }),
      ).rejects.toMatchObject({ code: 'E_VALIDATION', params: { field: 'name' } })
    }
    expect(JSON.stringify(state.library)).toBe(beforeRows)
    expect(state.activity).toHaveLength(beforeJournal)
  })

  it('refuses invalid level operations without changing rows or journaling', async () => {
    seedStructureLibrary()
    const state = mockState()
    const beforeRows = JSON.stringify(state.library)
    const beforeJournal = state.activity.length
    for (const level of [1, 7, -1, 256, 300]) {
      await expect(
        commands.headingUpdate(H_A, { mode: 'change_level', value: level }),
      ).rejects.toMatchObject({
        code: 'E_VALIDATION',
        params: { field: 'level', level: String(level), index: '0' },
      })
    }
    for (const delta of [-1, 2, 128, -129, 300]) {
      await expect(
        commands.headingUpdate(H_A, { mode: 'shift_branch', value: delta }),
      ).rejects.toMatchObject({ code: 'E_VALIDATION', params: { field: 'branch_shift' } })
    }
    expect(JSON.stringify(state.library)).toBe(beforeRows)
    expect(state.activity).toHaveLength(beforeJournal)
  })

  it('physically deletes a heading while keeping instruction order and dense positions', async () => {
    seedStructureLibrary()
    const state = mockState()
    const beforeJournal = state.activity.length
    await expect(commands.headingDelete(H_B)).resolves.toBeNull()
    expect(ids()).toEqual([H_A, I_A, I_B, H_C, I_C])
    expect(state.library.map((row) => row.position)).toEqual([1, 2, 3, 4, 5])
    expect(state.activity.slice(beforeJournal)).toHaveLength(1)
    expect(state.activity[state.activity.length - 1]).toMatchObject({
      action: 'heading.deleted',
      entity_type: 'heading',
      entity_id: H_B,
    })
    await expect(commands.headingDelete(I_A)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'id' },
    })
  })

  it('moves an instruction after a row or to the end of a heading section', async () => {
    seedStructureLibrary()
    await commands.itemsMove(I_A, { mode: 'after_item', id: I_C })
    expect(ids()).toEqual([H_A, H_B, I_B, H_C, I_C, I_A])
    await commands.itemsMove(I_A, { mode: 'into_heading', id: H_A })
    expect(ids()).toEqual([H_A, H_B, I_B, I_A, H_C, I_C])
    expect(mockState().library.map((row) => row.position)).toEqual([1, 2, 3, 4, 5, 6])
  })

  it('moves a branch as the target heading’s last child and aligns every heading level', async () => {
    seedStructureLibrary()
    await commands.itemsMove(H_A, { mode: 'into_heading', id: H_C })
    expect(ids()).toEqual([H_C, I_C, H_A, I_A, H_B, I_B])
    expect(heading(H_A)?.headingLevel).toBe(3)
    expect(heading(H_B)?.headingLevel).toBe(4)
    expect(heading(H_C)?.headingLevel).toBe(2)
  })

  it('moves a branch to the root at level two, preserving relative depth', async () => {
    seedStructureLibrary()
    const a = heading(H_A)
    const b = heading(H_B)
    if (!a || !b) throw new Error('seed headings must exist')
    a.headingLevel = 4
    b.headingLevel = 5
    await commands.itemsMove(H_A, { mode: 'root' })
    expect(ids()).toEqual([H_A, I_A, H_B, I_B, H_C, I_C])
    expect(heading(H_A)?.headingLevel).toBe(2)
    expect(heading(H_B)?.headingLevel).toBe(3)
  })

  it('moves a root with a zero level delta without restamping its heading', async () => {
    seedStructureLibrary()
    const source = heading(H_C)
    if (!source) throw new Error('seed heading must exist')
    const before = source.updatedAt
    await commands.itemsMove(H_C, { mode: 'root' })
    expect(ids()[0]).toBe(H_C)
    expect(heading(H_C)?.headingLevel).toBe(2)
    expect(heading(H_C)?.updatedAt).toBe(before)
  })

  it('aligns only the dragged pre-move range, not the adjacent closing section', async () => {
    seedStructureLibrary()
    const source = heading(H_A)
    const closer = heading(H_B)
    const target = heading(H_C)
    const iA = mockState().library.find((row) => row.id === I_A)
    const iB = mockState().library.find((row) => row.id === I_B)
    const iC = mockState().library.find((row) => row.id === I_C)
    if (!source || !closer || !target || !iA || !iB || !iC) {
      throw new Error('seed rows must exist')
    }
    target.headingLevel = 4
    const finalRoot = { ...source, id: 'structure-final-root', position: 5 }
    const finalItem = { ...iC, id: 'structure-final-item', position: 6 }
    const nested = { ...closer, id: 'structure-nested', position: 9 }
    const nestedItem = { ...iB, id: 'structure-nested-item', position: 10 }
    mockState().library = [
      { ...target, position: 1 },
      { ...iC, position: 2 },
      { ...closer, position: 3 },
      { ...iB, position: 4 },
      finalRoot,
      finalItem,
      { ...source, position: 7 },
      { ...iA, id: 'structure-source-item', position: 8 },
      nested,
      nestedItem,
    ]
    await expect(commands.itemsMove(H_A, { mode: 'into_heading', id: H_C })).resolves.toBeNull()
    expect(ids()).toEqual([
      H_C,
      I_C,
      H_A,
      'structure-source-item',
      'structure-nested',
      'structure-nested-item',
      H_B,
      I_B,
      'structure-final-root',
      'structure-final-item',
    ])
    expect(heading(H_A)?.headingLevel).toBe(5)
    expect(heading('structure-nested')?.headingLevel).toBe(6)
    expect(heading(H_B)?.headingLevel).toBe(3)
    expect(heading('structure-final-root')?.headingLevel).toBe(2)
  })

  it('rejects self/descendant drops and an automatic shift past level six', async () => {
    seedStructureLibrary()
    const state = mockState()
    const beforeRows = JSON.stringify(state.library)
    const beforeJournal = state.activity.length
    for (const target of [
      { mode: 'into_heading' as const, id: H_A },
      { mode: 'into_heading' as const, id: H_B },
      { mode: 'after_item' as const, id: I_A },
    ]) {
      await expect(commands.itemsMove(H_A, target)).rejects.toMatchObject({
        code: 'E_VALIDATION',
        params: { field: 'target' },
      })
    }
    expect(JSON.stringify(state.library)).toBe(beforeRows)
    const target = heading(H_C)
    const source = heading(H_A)
    const nested = heading(H_B)
    if (!target || !source || !nested) throw new Error('seed headings must exist')
    target.headingLevel = 5
    source.headingLevel = 5
    nested.headingLevel = 6
    const beforeOverflow = JSON.stringify(state.library)
    await expect(commands.itemsMove(H_A, { mode: 'into_heading', id: H_C })).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'heading_level' },
    })
    expect(JSON.stringify(state.library)).toBe(beforeOverflow)
    expect(state.activity).toHaveLength(beforeJournal)
  })

  it('journals nothing for an identity move', async () => {
    seedStructureLibrary()
    const state = mockState()
    const beforeRows = JSON.stringify(state.library)
    const beforeJournal = state.activity.length
    await expect(commands.itemsMove(H_A, { mode: 'root' })).resolves.toBeNull()
    expect(JSON.stringify(state.library)).toBe(beforeRows)
    expect(state.activity).toHaveLength(beforeJournal)
  })
})

// ---------------------------------------------------------------------------
// The five variant commands (§7.10 Library module, T-329): the mock twins
// of the Rust battery — every absorbed card's definition of done over the
// in-memory library rows, with the journal census read back through
// `state.activity`.
// ---------------------------------------------------------------------------

describe('variantCreate', () => {
  it('copies the main variant’s text in copy_main mode and never takes the flag', async () => {
    const state = mockState()
    const instruction = firstInstructionOf(state)
    const main = mainVariantOf(instruction)

    const created = await commands.variantCreate(instruction.id, 'Copy', 'copy_main')

    expect(created).toMatchObject({
      instruction_id: instruction.id,
      name: 'Copy',
      content: main.content,
      is_main: false,
      state: 'active',
    })
    // The copy is a NEW row; the main keeps its flag (DB-010).
    expect(instruction.variants.filter((row) => row.isMain)).toHaveLength(1)
    expect(state.activity[state.activity.length - 1]).toMatchObject({
      action: 'variant.created',
      entity_type: 'variant',
      entity_id: created.id,
    })
  })

  it('copies the CURRENT main’s text after the flag has moved', async () => {
    // The P2-4 twin: the copy source is the main AS IT STANDS, not any
    // variant — the flag moves, the text changes, and the copy must
    // carry the new main's text.
    const state = mockState()
    const instruction = firstInstructionOf(state)
    // `copy_main`: the promoted operand must carry real text —
    // LIB-036 refuses an empty promotion (the refusal twin lives in
    // the variantSetMain suite below).
    const second = await commands.variantCreate(instruction.id, 'Second', 'copy_main')
    await commands.variantSetMain(second.id)
    await commands.variantUpdate(second.id, null, 'the new main text')

    const copy = await commands.variantCreate(instruction.id, 'Copy', 'copy_main')

    expect(copy.content).toBe('the new main text')
  })

  it('refuses any create when the instruction has no active main (DB-010)', async () => {
    // The store rejects EVERY non-main insert without an active main —
    // both sources alike (the Rust command's `MainRequired` arm). The
    // broken state is reached by tampering the row (the tests' own
    // reachability rule).
    const state = mockState()
    const instruction = firstInstructionOf(state)
    const main = mainVariantOf(instruction)
    main.isMain = false

    await expect(commands.variantCreate(instruction.id, 'Empty', 'empty')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'instruction_id',
        reason: 'the instruction has no active main variant (DB-010)',
      },
    })
    await expect(commands.variantCreate(instruction.id, 'Copy', 'copy_main')).rejects.toMatchObject(
      {
        code: 'E_VALIDATION',
        params: { field: 'instruction_id' },
      },
    )
    // The trim is persisted (§6.9): a padded-but-legal name is stored
    // trimmed, exactly like the validator's own returned value.
    main.isMain = true
    const padded = await commands.variantCreate(instruction.id, '  Padded  ', 'empty')
    expect(padded.name).toBe('Padded')
  })

  it('answers the empty text in empty mode (LIB-014/015)', async () => {
    const state = mockState()
    const instruction = firstInstructionOf(state)

    const created = await commands.variantCreate(instruction.id, 'Empty', 'empty')

    expect(created.content).toBe('')
    expect(created.state).toBe('active')
  })

  it('refuses the NOCASE twin of an active name, writing nothing', async () => {
    const state = mockState()
    const instruction = firstInstructionOf(state)
    const before = instruction.variants.length
    const journalBefore = state.activity.length

    await expect(commands.variantCreate(instruction.id, 'MAIN', 'empty')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'variant_name' },
    })

    expect(instruction.variants).toHaveLength(before)
    expect(state.activity).toHaveLength(journalBefore)
  })

  it('applies the §6.9 name rules with the measured facts', async () => {
    const state = mockState()
    const instruction = firstInstructionOf(state)

    await expect(commands.variantCreate(instruction.id, '   ', 'empty')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'variant_name' },
    })
    await expect(
      commands.variantCreate(instruction.id, 'a'.repeat(101), 'empty'),
    ).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'variant_name', length: '101' },
    })
    await expect(commands.variantCreate(instruction.id, 'a\0b', 'empty')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'variant_name', index: '1' },
    })
    // The legal edge passes.
    await expect(
      commands.variantCreate(instruction.id, 'a'.repeat(100), 'empty'),
    ).resolves.toMatchObject({ name: 'a'.repeat(100) })
  })
})

describe('variantUpdate', () => {
  it('folds CRLF to LF and journals one changed entry', async () => {
    const state = mockState()
    const main = mainVariantOf(firstInstructionOf(state))

    const updated = await commands.variantUpdate(main.id, null, 'Be brief.\r\nSecond\r\n')

    expect(updated.content).toBe('Be brief.\nSecond\n')
    expect(state.activity[state.activity.length - 1]).toMatchObject({
      action: 'variant.changed',
      entity_type: 'variant',
      entity_id: main.id,
    })
  })

  it('refuses the reserved marker line with E_CONTENT_RESERVED', async () => {
    const state = mockState()
    const main = mainVariantOf(firstInstructionOf(state))
    const stored = main.content

    await expect(
      commands.variantUpdate(main.id, null, 'x\n<!-- EUNOMIASYNC:END -->'),
    ).rejects.toMatchObject({
      code: 'E_CONTENT_RESERVED',
      params: { field: 'variant_text', line: '2', marker: '<!-- EUNOMIASYNC:END -->' },
    })
    expect(main.content).toBe(stored)
  })

  it('catches a marker hidden behind a leading BOM at the build form’s own line', async () => {
    // P2-3's twin: `normalize::variant_text` strips the leading BOM and
    // blank lines before the DB-016 scan, so the marker IS caught (and
    // its line number counts from the stripped form).
    const state = mockState()
    const main = mainVariantOf(firstInstructionOf(state))

    await expect(
      commands.variantUpdate(main.id, null, '\uFEFF<!-- EUNOMIASYNC:BEGIN -->'),
    ).rejects.toMatchObject({
      code: 'E_CONTENT_RESERVED',
      params: { line: '1', marker: '<!-- EUNOMIASYNC:BEGIN -->' },
    })
    await expect(
      commands.variantUpdate(main.id, null, '\n\n<!-- EUNOMIASYNC:END -->'),
    ).rejects.toMatchObject({
      code: 'E_CONTENT_RESERVED',
      params: { line: '1' },
    })
    // The whitespace classes are ASCII exactly like the backend's
    // (`char::is_ascii_whitespace`): a trailing form-feed trims (the
    // marker line is caught), and a leading NBSP line is NOT empty (the
    // marker stays on line 2 — Unicode trim would drop it to 1).
    await expect(
      commands.variantUpdate(main.id, null, '<!-- EUNOMIASYNC:END -->\f'),
    ).rejects.toMatchObject({
      code: 'E_CONTENT_RESERVED',
      params: { line: '1' },
    })
    await expect(
      commands.variantUpdate(main.id, null, '\u00A0\n<!-- EUNOMIASYNC:END -->'),
    ).rejects.toMatchObject({
      code: 'E_CONTENT_RESERVED',
      params: { line: '2' },
    })
  })

  it('enforces the 262144-character limit at both edges', async () => {
    const state = mockState()
    const main = mainVariantOf(firstInstructionOf(state))

    await expect(commands.variantUpdate(main.id, null, 'a'.repeat(262_144))).resolves.toMatchObject(
      { content: 'a'.repeat(262_144) },
    )
    await expect(commands.variantUpdate(main.id, null, 'a'.repeat(262_145))).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'variant_text', length: '262145' },
    })
  })

  it('applies the §6.9 name rules with the measured facts', async () => {
    const state = mockState()
    const main = mainVariantOf(firstInstructionOf(state))

    await expect(commands.variantUpdate(main.id, '   ', null)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'variant_name' },
    })
    await expect(commands.variantUpdate(main.id, 'a'.repeat(101), null)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'variant_name', length: '101' },
    })
    // The positional-scan order (the validator's own): NUL before the
    // later break, both reported at the first offender's index, with
    // the character's stable name like the Rust mapper's.
    await expect(commands.variantUpdate(main.id, 'a\0b\nc', null)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'variant_name',
        reason: 'a variant name must not contain NUL',
        index: '1',
        character: 'NUL',
      },
    })
    await expect(commands.variantUpdate(main.id, 'a\nb', null)).rejects.toMatchObject({
      params: { reason: 'a variant name is one line', index: '1', character: 'LF' },
    })
    await expect(commands.variantUpdate(main.id, 'a\rb', null)).rejects.toMatchObject({
      params: { reason: 'a variant name is one line', index: '1', character: 'CR' },
    })
    // The trim is persisted on the rename path (§6.9).
    const renamed = await commands.variantUpdate(main.id, '  Renamed  ', null)
    expect(renamed.name).toBe('Renamed')
    const liveMain = liveVariantOf(state, main.id)
    if (!liveMain) throw new Error('the renamed row must still be in the store')
    expect(liveMain.name).toBe('Renamed')
  })

  it('answers a save without changes with nothing journaled (ALG-070)', async () => {
    const state = mockState()
    const main = mainVariantOf(firstInstructionOf(state))
    const journalBefore = state.activity.length

    const fieldless = await commands.variantUpdate(main.id, null, null)
    expect(fieldless.content).toBe(main.content)

    const sameValues = await commands.variantUpdate(main.id, main.name, main.content)
    expect(sameValues.content).toBe(main.content)

    expect(state.activity).toHaveLength(journalBefore)
  })
})

describe('variantSetMain', () => {
  it('leaves exactly one active main and demotes the previous one', async () => {
    const state = mockState()
    const instruction = firstInstructionOf(state)
    const main = activeVariantOf(instruction)
    // `copy_main`: the promoted operand carries real text — LIB-036
    // refuses an empty promotion (pinned by the twin below).
    const created = await commands.variantCreate(instruction.id, 'Second', 'copy_main')

    const promoted = await commands.variantSetMain(created.id)

    expect(promoted.is_main).toBe(true)
    if (main) expect(main.isMain).toBe(false)
    expect(instruction.variants.filter((row) => row.isMain)).toHaveLength(1)
    expect(state.activity[state.activity.length - 1]).toMatchObject({
      action: 'variant.changed',
      entity_id: created.id,
    })
  })

  it('answers re-promoting the current main as the journaled-nothing no-op', async () => {
    const state = mockState()
    const main = mainVariantOf(firstInstructionOf(state))
    const journalBefore = state.activity.length

    const answer = await commands.variantSetMain(main.id)

    expect(answer.is_main).toBe(true)
    expect(state.activity).toHaveLength(journalBefore)
  })

  it('refuses an empty and a whitespace-only promotion, writing nothing (LIB-036)', async () => {
    // The defensive surface behind the editor's disabled star: the
    // D-13/ALG-002 `outputsNoText` oracle (the same one
    // `variantGet`'s `is_empty` flag derives) decides, the refusal
    // is the typed `E_VALIDATION` naming the id, and a refused
    // promotion writes nothing — flags stay, the journal census
    // stays. The whitespace-only body arrives through the real save
    // (§6.9 allows it) and must count exactly as it counts in a
    // built file.
    const state = mockState()
    const instruction = firstInstructionOf(state)
    const main = mainVariantOf(instruction)
    const blank = await commands.variantCreate(instruction.id, 'Blank', 'empty')
    const padded = await commands.variantCreate(instruction.id, 'Padded', 'empty')
    await commands.variantUpdate(padded.id, null, ' \n\t ')
    const journalBefore = state.activity.length

    await expect(commands.variantSetMain(blank.id)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'id',
        value: blank.id,
        reason: 'an empty variant cannot be made the main variant',
      },
    })
    await expect(commands.variantSetMain(padded.id)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'id', reason: 'an empty variant cannot be made the main variant' },
    })

    // Nothing written: the main keeps its flag, neither empty row
    // gained it, exactly one main remains, the journal did not move.
    expect(main.isMain).toBe(true)
    expect(liveVariantOf(state, blank.id)?.isMain).toBe(false)
    expect(liveVariantOf(state, padded.id)?.isMain).toBe(false)
    expect(instruction.variants.filter((row) => row.isMain)).toHaveLength(1)
    expect(state.activity).toHaveLength(journalBefore)
  })

  it('still promotes real text after a refusal, and an emptied current main stays the no-op', async () => {
    // The refusal does not wedge the instruction: a real-text
    // sibling still demotes-and-promotes. And LIB-036's badge rule —
    // "an already-main variant keeps its badge while empty" — pins
    // the ORDER: the current-main no-op answers before the
    // emptiness refusal can (the emptied main is re-sent and
    // converges, never errors).
    const state = mockState()
    const instruction = firstInstructionOf(state)
    const main = mainVariantOf(instruction)
    const created = await commands.variantCreate(instruction.id, 'Second', 'copy_main')
    const empty = await commands.variantCreate(instruction.id, 'Doomed', 'empty')
    await expect(commands.variantSetMain(empty.id)).rejects.toMatchObject({
      code: 'E_VALIDATION',
    })

    const promoted = await commands.variantSetMain(created.id)
    expect(promoted.is_main).toBe(true)
    expect(instruction.variants.filter((row) => row.isMain)).toHaveLength(1)
    if (main) expect(main.isMain).toBe(false)

    // The emptied-main no-op: the save itself journals one real
    // change (the census baseline); the re-send must add nothing
    // on top of it. The premise is proven through the read — the
    // promoted row really is empty when the no-op answers (the
    // Rust twin asserts the same premise).
    await commands.variantUpdate(created.id, null, '')
    expect((await commands.variantGet(created.id)).is_empty).toBe(true)
    const censusAfterTheSave = state.activity.length

    const answer = await commands.variantSetMain(created.id)

    expect(answer.is_main).toBe(true)
    expect(state.activity).toHaveLength(censusAfterTheSave)
  })
})

describe('variantDelete', () => {
  it('moves every assignment to the default main — none disappears', async () => {
    const state = mockState()
    const instruction = firstInstructionOf(state)
    const main = activeVariantOf(instruction)
    const created = await commands.variantCreate(instruction.id, 'Second', 'empty')
    // The assignment rows the delete moves: two projects pointed at the
    // doomed variant (the store-level world the real command pins).
    const projects = state.projects.filter((row) => row.state === 'active').slice(0, 2)
    for (const project of projects) {
      state.assignments.push({
        projectId: project.id,
        instructionId: instruction.id,
        target: { kind: 'variant', value: created.id },
        createdAt: '2026-11-20T00:00:00.000Z',
        updatedAt: '2026-11-20T00:00:00.000Z',
      })
    }
    const before = state.assignments.filter((row) => row.instructionId === instruction.id).length

    const deleted = await commands.variantDelete(created.id, null)

    expect(deleted.state).toBe('deleted')
    expect(deleted.is_main).toBe(false)
    const after = state.assignments.filter((row) => row.instructionId === instruction.id)
    expect(after).toHaveLength(before)
    if (main) {
      expect(
        after.every((row) => row.target.kind === 'variant' && row.target.value === main.id),
      ).toBe(true)
    }
    expect(state.activity[state.activity.length - 1]).toMatchObject({
      action: 'variant.deleted',
      entity_id: created.id,
      details: JSON.stringify({ changed: 2 }),
    })
  })

  it('refuses the last active variant whatever the replacement names', async () => {
    const state = mockState()
    const main = mainVariantOf(firstInstructionOf(state))
    const journalBefore = state.activity.length

    await expect(commands.variantDelete(main.id, null)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'id',
        reason: 'the last active variant of an instruction cannot be deleted',
      },
    })
    // The named arm is the LIB-025 guard's own refusal (field `id`), not
    // the replacement-shape one — the guard runs first, exactly like the
    // store's check order.
    await expect(commands.variantDelete(main.id, main.id)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'id',
        reason: 'the last active variant of an instruction cannot be deleted',
      },
    })
    expect(main.state).toBe('active')
    expect(state.activity).toHaveLength(journalBefore)
  })

  it('requires a successor for the main and promotes it (DB-010 keeps one)', async () => {
    const state = mockState()
    const instruction = firstInstructionOf(state)
    const main = activeVariantOf(instruction)
    if (!main) throw new Error('the fixture instruction must have its Main')
    const created = await commands.variantCreate(instruction.id, 'Second', 'empty')

    await expect(commands.variantDelete(main.id, null)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'replacement' },
    })

    const deleted = await commands.variantDelete(main.id, created.id)
    expect(deleted.state).toBe('deleted')
    // `created` is the create-time wire snapshot; the live row is the
    // store's — the successor's promotion lives there.
    const liveCreated = liveVariantOf(state, created.id)
    if (!liveCreated) throw new Error('the created variant must still be in the store')
    expect(liveCreated.isMain).toBe(true)
    expect(instruction.variants.filter((row) => row.isMain)).toHaveLength(1)
  })
})

describe('variantRestore', () => {
  it('returns a soft-deleted variant to active without the main flag', async () => {
    const state = mockState()
    const instruction = firstInstructionOf(state)
    const created = await commands.variantCreate(instruction.id, 'Second', 'empty')
    await commands.variantDelete(created.id, null)

    const restored = await commands.variantRestore(created.id)

    expect(restored.state).toBe('active')
    expect(restored.deleted_at).toBeNull()
    expect(restored.is_main).toBe(false)
    expect(state.activity[state.activity.length - 1]).toMatchObject({
      action: 'variant.restored',
      entity_id: created.id,
    })
  })

  it('refuses a name reused by an active twin (E_VALIDATION)', async () => {
    const state = mockState()
    const instruction = firstInstructionOf(state)
    const created = await commands.variantCreate(instruction.id, 'Second', 'empty')
    await commands.variantDelete(created.id, null)
    const twin = await commands.variantCreate(instruction.id, 'SECOND', 'empty')

    await expect(commands.variantRestore(created.id)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'variant_name' },
    })

    // `created` is the create-time wire snapshot; the live rows are the
    // store's — the delete's effect lives there.
    const liveCreated = liveVariantOf(state, created.id)
    const liveTwin = liveVariantOf(state, twin.id)
    if (!liveCreated || !liveTwin) throw new Error('both rows must still be in the store')
    expect(liveCreated.state).toBe('deleted')
    expect(liveTwin.state).toBe('active')
  })

  it('answers restoring an active variant as the journaled-nothing no-op', async () => {
    const state = mockState()
    const main = mainVariantOf(firstInstructionOf(state))
    const journalBefore = state.activity.length

    const answer = await commands.variantRestore(main.id)

    expect(answer.state).toBe('active')
    expect(state.activity).toHaveLength(journalBefore)
  })
})

// ---------------------------------------------------------------------------
// `variant_get` (§7.10 Library module, T-351/D-198): the mock twin of the
// Rust battery — the stored row field by field (active and soft-deleted
// alike), the derived `is_empty` flag, the unknown-id refusal, and the
// never-truncated text. Reads journal nothing: the census stays put.
// ---------------------------------------------------------------------------

describe('variantGet (T-351)', () => {
  it('answers the stored row field by field, journaling nothing', async () => {
    const state = mockState()
    const instruction = firstInstructionOf(state)
    // `copy_main`: a non-main row carrying REAL text — the field set
    // reads unambiguously (is_main false, is_empty false).
    const alt = await commands.variantCreate(instruction.id, 'Alt', 'copy_main')
    const row = liveVariantOf(state, alt.id)
    if (!row) throw new Error('the created variant must exist')
    const journalBefore = state.activity.length

    const answer = await commands.variantGet(alt.id)

    // D-198's field-for-field contract, checked against the live row
    // (the mock store's own truth), every field at once.
    expect(answer).toEqual({
      id: row.id,
      instruction_id: instruction.id,
      name: row.name,
      content: row.content,
      is_main: row.isMain,
      state: row.state,
      created_at: row.createdAt,
      updated_at: row.updatedAt,
      deleted_at: row.deletedAt,
      is_empty: false,
    })
    expect(answer.content).not.toBe('')
    expect(state.activity).toHaveLength(journalBefore)
  })

  it('answers a soft-deleted variant as stored (D-198)', async () => {
    const state = mockState()
    const instruction = firstInstructionOf(state)
    const alt = await commands.variantCreate(instruction.id, 'Alt', 'empty')
    await commands.variantDelete(alt.id, null)
    const row = liveVariantOf(state, alt.id)
    if (!row) throw new Error('the soft-deleted variant must still exist')

    const answer = await commands.variantGet(alt.id)

    expect(answer.state).toBe('deleted')
    expect(answer.deleted_at).toBe(row.deletedAt)
    expect(answer.deleted_at).not.toBeNull()
    expect(answer.content).toBe(row.content)
    expect(answer.is_main).toBe(row.isMain)
    expect(answer.name).toBe(row.name)
  })

  it('marks empty and whitespace-only text empty, real text not (ALG-002)', async () => {
    const state = mockState()
    const instruction = firstInstructionOf(state)
    const blank = await commands.variantCreate(instruction.id, 'Blank', 'empty')
    const padded = await commands.variantCreate(instruction.id, 'Padded', 'empty')
    await commands.variantUpdate(padded.id, null, ' \n\t ')

    expect((await commands.variantGet(blank.id)).is_empty).toBe(true)
    expect((await commands.variantGet(padded.id)).is_empty).toBe(true)
    const main = mainVariantOf(instruction)
    expect((await commands.variantGet(main.id)).is_empty).toBe(false)
  })

  it('refuses an unknown id with the typed E_VALIDATION naming id', async () => {
    const state = mockState()
    const heading = state.library.find((row) => row.kind === 'heading')
    if (!heading) throw new Error('the fixture world must hold a heading')

    await expect(commands.variantGet('no-such-variant')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'id', reason: 'no variant with this id' },
    })
    // A heading's id names no variant row either (MAIN-001's
    // counter-example): the same by-id refusal.
    await expect(commands.variantGet(heading.id)).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'id' },
    })
  })

  it('never truncates the full text (DB-006)', async () => {
    const state = mockState()
    const instruction = firstInstructionOf(state)
    const long = `${'a'.repeat(600)}${'b'.repeat(400)}`
    const variant = await commands.variantCreate(instruction.id, 'Long', 'empty')
    await commands.variantUpdate(variant.id, null, long)

    const answer = await commands.variantGet(variant.id)

    expect(answer.content).toBe(long)
    expect(answer.content).toHaveLength(1000)
    expect(answer.content.endsWith('b'.repeat(400))).toBe(true)
  })
})

describe('assignmentSet (T-331)', () => {
  it('changes one project only, defaults to Main, and journals only real changes', async () => {
    const state = mockState()
    const instruction = firstInstructionOf(state)
    const main = mainVariantOf(instruction)
    const alt = await commands.variantCreate(instruction.id, 'Alt', 'empty')
    const projects = state.projects.filter((row) => row.state === 'active').slice(0, 2)
    const [p1, p2] = projects
    if (!p1 || !p2) throw new Error('two active projects must exist')
    await commands.assignmentSet(p1.id, instruction.id, true, null)
    await commands.assignmentSet(p2.id, instruction.id, true, null)
    const p2Before = state.assignments.find(
      (row) => row.projectId === p2.id && row.instructionId === instruction.id,
    )
    const journalBefore = state.activity.length

    const changed = await commands.assignmentSet(p1.id, instruction.id, true, alt.id)

    expect(changed?.target).toEqual({ kind: 'variant', value: alt.id })
    expect(
      state.assignments.find(
        (row) => row.projectId === p2.id && row.instructionId === instruction.id,
      ),
    ).toEqual(p2Before)
    expect(state.activity).toHaveLength(journalBefore + 1)
    expect(state.activity[state.activity.length - 1]).toMatchObject({
      action: 'assignment.changed',
      project_id: p1.id,
      details: JSON.stringify({ changed: 1 }),
    })

    const same = await commands.assignmentSet(p1.id, instruction.id, true, alt.id)
    expect(same).toEqual(changed)
    expect(state.activity).toHaveLength(journalBefore + 1)
    expect(await commands.assignmentSet(p1.id, instruction.id, false, null)).toBeNull()
    expect(state.activity).toHaveLength(journalBefore + 2)
    expect(await commands.assignmentSet(p1.id, instruction.id, false, null)).toBeNull()
    expect(state.activity).toHaveLength(journalBefore + 2)
    expect(main.state).toBe('active')
  })

  it('preserves an already-enabled custom cell on a null-variant re-send', async () => {
    const state = mockState()
    state.assignments = []
    const instruction = firstInstructionOf(state)
    const main = mainVariantOf(instruction)
    const project = state.projects.find((row) => row.state === 'active')
    if (!project) throw new Error('an active project must exist')
    await commands.assignmentSetCustom(project.id, instruction.id, 'keep custom', null)
    const journalBefore = state.activity.length

    const unchanged = await commands.assignmentSet(project.id, instruction.id, true, null)

    expect(unchanged?.target).toEqual({ kind: 'custom', value: 'keep custom' })
    expect(state.activity).toHaveLength(journalBefore)
    const converted = await commands.assignmentSet(project.id, instruction.id, true, main.id)
    expect(converted?.target).toEqual({ kind: 'variant', value: main.id })
    expect(state.activity).toHaveLength(journalBefore + 1)
  })

  it('refuses a variant on disable and unknown project/instruction ids', async () => {
    const state = mockState()
    const instruction = firstInstructionOf(state)
    const main = mainVariantOf(instruction)
    const project = state.projects.find((row) => row.state === 'active')
    if (!project) throw new Error('an active project must exist')
    await expect(
      commands.assignmentSet(project.id, instruction.id, false, main.id),
    ).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'variant_id' },
    })
    await expect(
      commands.assignmentSet('missing', instruction.id, true, null),
    ).rejects.toMatchObject({
      params: { field: 'project_id' },
    })
    await expect(commands.assignmentSet(project.id, 'missing', true, null)).rejects.toMatchObject({
      params: { field: 'instruction_id' },
    })
  })
})

describe('assignmentSetCustom (T-331)', () => {
  it('updates one assignment without forking or changing the shared variant', async () => {
    const state = mockState()
    const instruction = firstInstructionOf(state)
    const main = mainVariantOf(instruction)
    const projects = state.projects.filter((row) => row.state === 'active').slice(0, 2)
    const [p1, p2] = projects
    if (!p1 || !p2) throw new Error('two active projects must exist')
    await commands.assignmentSet(p2.id, instruction.id, true, null)
    const p2Before = state.assignments.find((row) => row.projectId === p2.id)
    const variantCount = instruction.variants.length
    const sharedBefore = { ...main }
    const journalBefore = state.activity.length

    const first = await commands.assignmentSetCustom(p1.id, instruction.id, 'project one\r\n', null)

    expect(first?.target).toEqual({ kind: 'custom', value: 'project one\n' })
    expect(instruction.variants).toHaveLength(variantCount)
    expect(main).toEqual(sharedBefore)
    expect(state.assignments.find((row) => row.projectId === p2.id)).toEqual(p2Before)
    expect(state.activity).toHaveLength(journalBefore + 1)
    expect(
      await commands.assignmentSetCustom(p1.id, instruction.id, 'project one\n', null),
    ).toEqual(first)
    expect(state.activity).toHaveLength(journalBefore + 1)

    const second = await commands.assignmentSetCustom(
      p1.id,
      instruction.id,
      'project one updated',
      null,
    )
    expect(second?.created_at).toBe(first?.created_at)
    expect(second?.target).toEqual({ kind: 'custom', value: 'project one updated' })
    expect(instruction.variants).toHaveLength(variantCount)
    const customLabels = (await commands.libraryGet(null)).tree
    const customLabel = customLabels.find(
      (entry) => entry.kind === 'instruction' && entry.id === instruction.id,
    )
    expect(
      customLabel?.kind === 'instruction'
        ? customLabel.project_labels.find((label) => label.project_id === p1.id)?.variant_id
        : undefined,
    ).toBeNull()

    const cleared = await commands.assignmentSetCustom(p1.id, instruction.id, null, null)
    expect(cleared?.target).toEqual({ kind: 'variant', value: main.id })
    expect(state.activity).toHaveLength(journalBefore + 3)

    const labels = (await commands.libraryGet(null)).tree
    const labeled = labels.find(
      (entry) => entry.kind === 'instruction' && entry.id === instruction.id,
    )
    expect(
      labeled?.kind === 'instruction'
        ? labeled.project_labels.find((label) => label.project_id === p2.id)?.variant_id
        : undefined,
    ).toBe(main.id)
  })

  it('validates custom text before writing and never echoes it', async () => {
    const state = mockState()
    state.assignments = []
    const instruction = firstInstructionOf(state)
    const project = state.projects.find((row) => row.state === 'active')
    if (!project) throw new Error('an active project must exist')
    const journalBefore = state.activity.length
    for (const content of ['bad\0text', 'x'.repeat(262_145)]) {
      await expect(
        commands.assignmentSetCustom(project.id, instruction.id, content, null),
      ).rejects.toMatchObject({ code: 'E_VALIDATION', params: { field: 'custom_text' } })
    }
    await expect(
      commands.assignmentSetCustom(
        project.id,
        instruction.id,
        'text\n<!-- EUNOMIASYNC:BEGIN -->',
        null,
      ),
    ).rejects.toMatchObject({
      code: 'E_CONTENT_RESERVED',
      params: { field: 'custom_text', line: '2' },
    })
    expect(state.activity).toHaveLength(journalBefore)
    expect(
      state.assignments.some(
        (row) => row.projectId === project.id && row.instructionId === instruction.id,
      ),
    ).toBe(false)
    expect(
      (await commands.assignmentSetCustom(project.id, instruction.id, '', null))?.target,
    ).toEqual({
      kind: 'custom',
      value: '',
    })
  })
})

describe('assignmentSetCustom extend (T-356, MAIN-034)', () => {
  it("keeps the cell's variant as the base, adopts the main for a variantless cell, and re-sends journal nothing", async () => {
    const state = mockState()
    const instruction = firstInstructionOf(state)
    const main = mainVariantOf(instruction)
    const alt = await commands.variantCreate(instruction.id, 'Alt', 'empty')
    const projects = state.projects.filter((row) => row.state === 'active').slice(0, 2)
    const [p1, p2] = projects
    if (!p1 || !p2) throw new Error('two active projects must exist')
    await commands.assignmentSet(p1.id, instruction.id, true, alt.id)
    const journalBefore = state.activity.length

    // The variant-backed cell keeps its variant as the base; the
    // supplement is folded like variant text.
    const kept = await commands.assignmentSetCustom(p1.id, instruction.id, 'plus\r\n', 'extend')
    expect(kept?.target).toEqual({
      kind: 'extend',
      value: { base: alt.id, supplement: 'plus\n' },
    })

    // The unassigned cell adopts the active main (MAIN-034: "the main
    // variant for an unassigned cell").
    const adopted = await commands.assignmentSetCustom(p2.id, instruction.id, 'plus two', 'extend')
    expect(adopted?.target).toEqual({
      kind: 'extend',
      value: { base: main.id, supplement: 'plus two' },
    })
    expect(state.activity).toHaveLength(journalBefore + 2)

    // A same-mode, same-content re-send journals nothing.
    const reSent = await commands.assignmentSetCustom(p1.id, instruction.id, 'plus\n', 'extend')
    expect(reSent).toEqual(kept)
    expect(state.activity).toHaveLength(journalBefore + 2)

    // The mock store rows carry the extend pair itself.
    expect(
      state.assignments.find(
        (row) => row.projectId === p1.id && row.instructionId === instruction.id,
      )?.target,
    ).toEqual({ kind: 'extend', value: { base: alt.id, supplement: 'plus\n' } })
  })

  it('switches modes both ways and clears to the main, not the base', async () => {
    const state = mockState()
    const instruction = firstInstructionOf(state)
    const main = mainVariantOf(instruction)
    const alt = await commands.variantCreate(instruction.id, 'Alt', 'empty')
    const project = state.projects.find((row) => row.state === 'active')
    if (!project) throw new Error('an active project must exist')
    await commands.assignmentSet(project.id, instruction.id, true, alt.id)

    const extend = await commands.assignmentSetCustom(
      project.id,
      instruction.id,
      'supplement',
      'extend',
    )
    expect(extend?.target).toEqual({
      kind: 'extend',
      value: { base: alt.id, supplement: 'supplement' },
    })

    // Extend → replace: the base reference is dropped, the text stays.
    const replaced = await commands.assignmentSetCustom(
      project.id,
      instruction.id,
      'whole cell',
      'replace',
    )
    expect(replaced?.target).toEqual({ kind: 'custom', value: 'whole cell' })

    // Replace → extend: no variant to keep, the main becomes the base.
    const extended = await commands.assignmentSetCustom(
      project.id,
      instruction.id,
      'supplement',
      'extend',
    )
    expect(extended?.target).toEqual({
      kind: 'extend',
      value: { base: main.id, supplement: 'supplement' },
    })

    // Clearing drops the mode with the text and restores the MAIN, not
    // the base (MAIN-007).
    const cleared = await commands.assignmentSetCustom(project.id, instruction.id, null, 'extend')
    expect(cleared?.target).toEqual({ kind: 'variant', value: main.id })
  })

  it('validates the supplement like replace text before writing', async () => {
    const state = mockState()
    state.assignments = []
    const instruction = firstInstructionOf(state)
    const project = state.projects.find((row) => row.state === 'active')
    if (!project) throw new Error('an active project must exist')
    const journalBefore = state.activity.length
    for (const content of ['bad\0text', 'x'.repeat(262_145)]) {
      await expect(
        commands.assignmentSetCustom(project.id, instruction.id, content, 'extend'),
      ).rejects.toMatchObject({ code: 'E_VALIDATION', params: { field: 'custom_text' } })
    }
    // DB-016's reserved-marker rule covers the supplement (MAIN-034).
    await expect(
      commands.assignmentSetCustom(
        project.id,
        instruction.id,
        'text\n<!-- EUNOMIASYNC:BEGIN -->',
        'extend',
      ),
    ).rejects.toMatchObject({
      code: 'E_CONTENT_RESERVED',
      params: { field: 'custom_text', line: '2' },
    })
    expect(state.activity).toHaveLength(journalBefore)
    expect(
      state.assignments.some(
        (row) => row.projectId === project.id && row.instructionId === instruction.id,
      ),
    ).toBe(false)
  })

  it('refuses an extend write with no active main instead of degrading to replace', async () => {
    // The twin of the store bug D-238 records as found-red: a cell
    // without a variant must adopt the active main, and DB-010 broken
    // out-of-band (the main flag cleared on the store's own row) is
    // the honest refusal — never a silent replace write.
    const state = mockState()
    state.assignments = []
    const instruction = firstInstructionOf(state)
    const main = mainVariantOf(instruction)
    main.isMain = false
    const project = state.projects.find((row) => row.state === 'active')
    if (!project) throw new Error('an active project must exist')
    const journalBefore = state.activity.length

    await expect(
      commands.assignmentSetCustom(project.id, instruction.id, 'supplement', 'extend'),
    ).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: {
        field: 'instruction_id',
        reason: 'the instruction has no active main variant (DB-010)',
      },
    })
    expect(
      state.assignments.some(
        (row) => row.projectId === project.id && row.instructionId === instruction.id,
      ),
    ).toBe(false)
    expect(state.activity).toHaveLength(journalBefore)
  })

  it('treats an extend cell as a custom cell: a null variant is a no-op, an explicit variant converts, bulk skips it', async () => {
    const state = mockState()
    state.assignments = []
    const instruction = firstInstructionOf(state)
    const main = mainVariantOf(instruction)
    const alt = await commands.variantCreate(instruction.id, 'Alt', 'empty')
    const projects = state.projects.filter((row) => row.state === 'active').slice(0, 2)
    const [p1, p2] = projects
    if (!p1 || !p2) throw new Error('two active projects must exist')
    await commands.assignmentSet(p1.id, instruction.id, true, alt.id)
    await commands.assignmentSet(p2.id, instruction.id, true, alt.id)
    await commands.assignmentSetCustom(p1.id, instruction.id, 'supplement', 'extend')
    await commands.assignmentSetCustom(p2.id, instruction.id, 'supplement', 'extend')
    const journalBefore = state.activity.length

    // A null variant is the verified no-op a custom cell gets.
    const noOp = await commands.assignmentSet(p1.id, instruction.id, true, null)
    expect(noOp?.target).toEqual({
      kind: 'extend',
      value: { base: alt.id, supplement: 'supplement' },
    })
    expect(state.activity).toHaveLength(journalBefore)

    // An explicit variant converts the cell: the supplement and the
    // mode are dropped.
    const converted = await commands.assignmentSet(p1.id, instruction.id, true, main.id)
    expect(converted?.target).toEqual({ kind: 'variant', value: main.id })
    expect(state.activity).toHaveLength(journalBefore + 1)

    // Bulk clear skips the extend cell.
    const bulk = await commands.assignmentsBulk([instruction.id], [p2.id], false, null)
    expect(bulk.changed).toBe(0)
    expect(bulk.already_assigned).toEqual([{ project_id: p2.id, instruction_id: instruction.id }])
    expect(state.assignments.find((row) => row.projectId === p2.id)?.target).toEqual({
      kind: 'extend',
      value: { base: alt.id, supplement: 'supplement' },
    })
  })

  it("variant_delete moves an extend cell's base and keeps the supplement", async () => {
    const state = mockState()
    state.assignments = []
    const instruction = firstInstructionOf(state)
    const main = mainVariantOf(instruction)
    const alt = await commands.variantCreate(instruction.id, 'Alt', 'empty')
    const project = state.projects.find((row) => row.state === 'active')
    if (!project) throw new Error('an active project must exist')
    await commands.assignmentSet(project.id, instruction.id, true, alt.id)
    await commands.assignmentSetCustom(project.id, instruction.id, 'supplement', 'extend')

    // The label carries the base before the delete (LIB-001).
    const labeledBefore = (await commands.libraryGet(null)).tree.find(
      (entry) => entry.kind === 'instruction' && entry.id === instruction.id,
    )
    expect(
      labeledBefore?.kind === 'instruction'
        ? labeledBefore.project_labels.find((label) => label.project_id === project.id)?.variant_id
        : undefined,
    ).toBe(alt.id)

    const journalBefore = state.activity.length
    await commands.variantDelete(alt.id, null)

    // The base moved to the main; the supplement stayed.
    expect(state.assignments.find((row) => row.projectId === project.id)?.target).toEqual({
      kind: 'extend',
      value: { base: main.id, supplement: 'supplement' },
    })
    // The move's Bulk payload counts the extend cell as affected
    // (LIB-023/MAIN-034).
    const deleteEntry = state.activity[state.activity.length - 1]
    expect(journalBefore).toBeLessThan(state.activity.length)
    expect(deleteEntry).toMatchObject({
      action: 'variant.deleted',
      entity_type: 'variant',
      entity_id: alt.id,
    })
    expect(JSON.parse(String(deleteEntry.details))).toEqual({ changed: 1 })
  })
})

describe('assignmentsBulk (T-331)', () => {
  it('deduplicates the product, preserves existing/custom cells, and clears only variant cells', async () => {
    const state = mockState()
    state.assignments = []
    const instruction = firstInstructionOf(state)
    const main = mainVariantOf(instruction)
    const alt = await commands.variantCreate(instruction.id, 'Alt', 'empty')
    const projects = state.projects.filter((row) => row.state === 'active').slice(0, 3)
    const [p1, p2, p3] = projects
    if (!p1 || !p2 || !p3) throw new Error('three active projects must exist')
    await commands.assignmentSet(p1.id, instruction.id, true, alt.id)
    await commands.assignmentSetCustom(p2.id, instruction.id, 'keep custom', null)
    const p1Before = state.assignments.find((row) => row.projectId === p1.id)
    const p2Before = state.assignments.find((row) => row.projectId === p2.id)
    const journalBefore = state.activity.length

    const assigned = await commands.assignmentsBulk(
      [instruction.id, instruction.id],
      [p1.id, p2.id, p3.id, p3.id],
      true,
      main.id,
    )

    expect(assigned.changed).toBe(1)
    expect(assigned.already_assigned).toEqual([
      { project_id: p1.id, instruction_id: instruction.id },
      { project_id: p2.id, instruction_id: instruction.id },
    ])
    expect(state.assignments.find((row) => row.projectId === p1.id)).toEqual(p1Before)
    expect(state.assignments.find((row) => row.projectId === p2.id)).toEqual(p2Before)
    expect(state.activity).toHaveLength(journalBefore + 1)
    expect(state.activity[state.activity.length - 1]).toMatchObject({
      entity_type: 'assignment',
      entity_id: null,
      project_id: null,
      details: JSON.stringify({ changed: 1 }),
    })

    const cleared = await commands.assignmentsBulk(
      [instruction.id],
      [p1.id, p2.id, p3.id],
      false,
      null,
    )
    expect(cleared.changed).toBe(2)
    expect(cleared.already_assigned).toEqual([
      { project_id: p2.id, instruction_id: instruction.id },
    ])
    expect(state.assignments.find((row) => row.projectId === p2.id)).toEqual(p2Before)
    expect(state.assignments.some((row) => row.projectId === p1.id)).toBe(false)
    expect(state.assignments.some((row) => row.projectId === p3.id)).toBe(false)
  })

  it('refuses a variant for multiple rows or a clear operation', async () => {
    const state = mockState()
    const [first, second] = state.library.filter((row) => row.kind === 'instruction')
    const project = state.projects.find((row) => row.state === 'active')
    if (!first || !second || !project) throw new Error('fixture rows must exist')
    await expect(
      commands.assignmentsBulk([first.id, 'missing'], [project.id], true, null),
    ).rejects.toMatchObject({ params: { field: 'instruction_id' } })
    // The shape guard runs before the database when a variant is supplied.
    await expect(
      commands.assignmentsBulk([first.id, second.id], [project.id], true, 'v'),
    ).rejects.toMatchObject({ params: { field: 'variant_id' } })
    await expect(
      commands.assignmentsBulk([first.id], [project.id], false, 'v'),
    ).rejects.toMatchObject({ params: { field: 'variant_id' } })
  })
})

describe('revisionsList / revisionGet (T-331)', () => {
  it('serves instruction and variant history newest-first and reads one full row', async () => {
    const state = mockState()
    const created = await commands.instructionCreate('History instruction', 'shared', null)
    await commands.variantUpdate(created.variant_id, null, 'updated')
    await commands.instructionHide(created.instruction_id)
    state.revisions.forEach((row, index) => {
      row.created_at = `2026-11-24T12:00:0${index + 1}.000Z`
    })
    const journalBefore = state.activity.length

    const history = await commands.revisionsList(created.instruction_id)

    expect(history.map((row) => row.op)).toEqual(['hide', 'update', 'create', 'create'])
    expect(history.map((row) => row.entity_type)).toEqual([
      'instruction',
      'variant',
      'variant',
      'instruction',
    ])
    expect(history.every((row) => /^[0-9a-f]{64}$/.test(row.snapshot_hash))).toBe(true)
    expect(history[0]).not.toHaveProperty('snapshot')
    const full = await commands.revisionGet(history[0]?.id ?? '')
    expect(full).toEqual(state.revisions.find((row) => row.id === history[0]?.id))
    expect(full.snapshot).toBe('{"name":"History instruction","state":"hidden"}')
    expect(state.activity).toHaveLength(journalBefore)
    await expect(commands.revisionGet('missing')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'id' },
    })
    const heading = state.library.find((row) => row.kind === 'heading')
    if (!heading) throw new Error('a heading must exist')
    await expect(commands.revisionsList(heading.id)).rejects.toMatchObject({
      params: {
        field: 'instruction_id',
        reason: 'a heading has no version history (LIB-026/6.2)',
      },
    })
  })
})

describe('backupsList (T-125)', () => {
  /** The first project's seeded stamp for row `n` (the seed's own
   * formula: the fixture era's base instant, 5 minutes in, one second
   * per row) — the literal oracle the ordering and the §5.5 name's
   * compact stamp both derive from. */
  const stampOf = (n: number): string => isoUtc(BASE_TS_MS + (300 + n) * 1000)

  it("serves a project's whole backup history — every status, newest first, run-orphaned null last", async () => {
    const state = mockState()
    const project = state.projects[0]!
    const archived = state.projects.find((row) => row.state === 'archived')
    if (!archived) throw new Error('the fixture world must hold an archived project')
    const journalBefore = state.activity.length

    const rows = await commands.backupsList(project.id)

    // The seeded history, newest first: the live copy, the limit-pruned
    // row, the missing copy, and the run-orphaned discarded row (its
    // §6.4-swept run leaves the honest null link).
    expect(rows.map((row) => row.status)).toEqual(['ok', 'pruned', 'missing', 'discarded'])
    expect(rows.map((row) => row.id)).toEqual([
      `${project.id}-bak-3`,
      `${project.id}-bak-2`,
      `${project.id}-bak-1`,
      `${project.id}-bak-0`,
    ])
    expect(rows.map((row) => row.sync_run_id)).toEqual([
      `${project.id}-run-3`,
      `${project.id}-run-2`,
      `${project.id}-run-1`,
      null,
    ])
    // The stamps pinned literally, newest first — the order cannot ride
    // on the id tiebreak alone.
    expect(rows.map((row) => row.created_at)).toEqual([3, 2, 1, 0].map(stampOf))
    // Every field of the row shape (BKP-004's set plus the status); the
    // §5.5 name's compact stamp is the row's own stamp in BKP-001's
    // `YYYYMMDDTHHMMSSmmmZ` form.
    for (const [index, row] of rows.entries()) {
      const n = rows.length - 1 - index
      expect(row.source_path).toBe(`${project.path}/AGENTS.md`)
      expect(row.backup_path).toBe(
        `backups/${project.id}__${project.path.split('/').pop()}/AGENTS.md.${stampOf(n).replace(/[-:.]/g, '')}.${`${n}`.padStart(8, '0')}.bak`,
      )
      expect(row.size_bytes).toBe(64 * (n + 1))
    }
    // Read-only and strictly scoped: no journal entry, and the archived
    // project's own history lists (DB-014 — the read is state-agnostic).
    expect(state.activity).toHaveLength(journalBefore)
    expect(await commands.backupsList(archived.id)).toHaveLength(4)
    // The unknown-id refusal and the known-empty answer.
    await expect(commands.backupsList('missing')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'project_id', reason: 'no project has this id' },
    })
    state.backupRecords = state.backupRecords.filter((row) => row.projectId !== project.id)
    await expect(commands.backupsList(project.id)).resolves.toEqual([])
  })
})

describe('backupSave (T-353)', () => {
  /** The first project's seeded stamp for row `n` (the seed's own
   * formula) — the oracle the suggested name's stamp derives from. */
  const stampOf = (n: number): string => isoUtc(BASE_TS_MS + (300 + n) * 1000)

  it('exports the ok copy, refuses the terminal statuses and the unknown id (BKP-012)', async () => {
    const state = mockState()
    const project = state.projects[0]!
    const journalBefore = state.activity.length

    // The ok row (the seed's `n: 3` copy) answers the would-be
    // destination: the suggested name's stem, stamp and extension over
    // the mock's canned exports root — `<stem>.backup-<stamp>.<ext>`
    // derived from the row's own §5.5 name.
    const saved = await commands.backupSave(`${project.id}-bak-3`)
    expect(saved).toBe(`/home/dev/exports/AGENTS.backup-${stampOf(3).replace(/[-:.]/g, '')}.md`)

    // The three terminal statuses refuse with E_RESTORE_SOURCE_MISSING
    // naming the record's status (the seed orders discarded(0),
    // missing(1), pruned(2), ok(3)).
    for (const [n, status] of [0, 1, 2].map((n) => [n, ['discarded', 'missing', 'pruned'][n]])) {
      await expect(commands.backupSave(`${project.id}-bak-${n}`)).rejects.toMatchObject({
        code: 'E_RESTORE_SOURCE_MISSING',
        message_key: 'errors.restore_source_missing',
        params: { status },
      })
    }

    // The unknown id is the typed refusal naming `id`.
    await expect(commands.backupSave('missing')).rejects.toMatchObject({
      code: 'E_VALIDATION',
      params: { field: 'id', reason: 'no backup record has this id' },
    })

    // Read-only: no journal entry, no row change, no new row.
    expect(state.activity).toHaveLength(journalBefore)
    expect(state.backupRecords.filter((row) => row.projectId === project.id)).toHaveLength(4)
  })
})

/** The first kind-narrowed instruction row of the fixture world — the
 * union model's parent row the variant tests operate on. */
function firstInstructionOf(state: ReturnType<typeof mockState>) {
  const row = state.library.find((item) => item.kind === 'instruction')
  if (!row || row.kind !== 'instruction') {
    throw new Error('the fixture world must hold an instruction')
  }
  return row
}

/** The instruction row's ACTIVE main variant — the fixture world's
 * `Main` (DB-010). */
function mainVariantOf(instruction: ReturnType<typeof firstInstructionOf>) {
  const main = instruction.variants.find((row) => row.isMain && row.state === 'active')
  if (!main) throw new Error('the fixture instruction must have its Main')
  return main
}

/** The live variant row behind a wire snapshot's id (the union's nested
 * lookup — a wire answer is a snapshot; the store's row is live). */
function liveVariantOf(state: ReturnType<typeof mockState>, variantId: string) {
  for (const row of state.library) {
    if (row.kind !== 'instruction') continue
    const variant = row.variants.find((candidate) => candidate.id === variantId)
    if (variant) return variant
  }
  return null
}

/** The first ACTIVE variant of the instruction — the fixture world's
 * `Main` (the union-narrowed `activeVariantOf`). */
function activeVariantOf(instruction: ReturnType<typeof firstInstructionOf>) {
  return instruction.variants.find((row) => row.state === 'active') ?? null
}
