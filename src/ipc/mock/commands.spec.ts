import { beforeEach, describe, expect, it } from 'vitest'
import { mockCommands } from './commands'
import { mockState, resetMockState } from './state'
import { mockEvents } from './events'
import { PALETTE, type FixtureProfile } from '../fixtures/generator'
import type { LibraryTreeItem } from '../contract'

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
    entry: LibraryTreeItem,
  ): entry is Extract<LibraryTreeItem, { kind: 'instruction' }> => entry.kind === 'instruction'

  /** Narrows the wire union to its heading arm. */
  const isHeading = (
    entry: LibraryTreeItem,
  ): entry is Extract<LibraryTreeItem, { kind: 'heading' }> => entry.kind === 'heading'

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
    const tree = await commands.libraryGet(null)
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
    const expected = new Map<string, { project_id: string; variant_id: string }[]>()
    for (const project of [...state.projects]
      .sort((a, b) => a.sortOrder - b.sortOrder)
      .filter((p) => p.state === 'active')) {
      for (const assignment of state.assignments.filter((a) => a.projectId === project.id)) {
        const list = expected.get(assignment.instructionId) ?? []
        list.push({ project_id: project.id, variant_id: assignment.variantId })
        expected.set(assignment.instructionId, list)
      }
    }
    const tree = await commands.libraryGet(null)
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
      const relisted = await commands.libraryGet(null)
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
      const tree = await commands.libraryGet(null)
      const served = tree.filter(isInstruction).find((instruction) => instruction.id === target.id)
      expect(served?.variants[0]?.is_empty, 'the empty variant is marked (LIB-014)').toBe(true)
      expect(served?.variants[0]?.is_main, 'the main flag rides the same list (DB-010)').toBe(true)
      // D-13's reading: whitespace-only text outputs no text, so it is
      // as empty as "" — the mock mirrors the backend's ALG-002-based
      // flag. The BOM rule is positional (round-2 P3-1): a trailing
      // BOM blocks the right-hand strip, a leading one behind blank
      // lines does not.
      target.variants[0]!.content = ' \n\t '
      const whitespace = await commands.libraryGet(null)
      const servedBlank = whitespace
        .filter(isInstruction)
        .find((instruction) => instruction.id === target.id)
      expect(servedBlank?.variants[0]?.is_empty, 'whitespace-only outputs no text (D-13)').toBe(
        true,
      )
      target.variants[0]!.content = ' \u{FEFF}'
      const trailingBom = await commands.libraryGet(null)
      const servedTrailing = trailingBom
        .filter(isInstruction)
        .find((instruction) => instruction.id === target.id)
      expect(
        servedTrailing?.variants[0]?.is_empty,
        'a TRAILING BOM blocks the strip: the variant outputs text',
      ).toBe(false)
      target.variants[0]!.content = '\n\u{FEFF}'
      const leadingBom = await commands.libraryGet(null)
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
    const tree = await commands.libraryGet(null)
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
      headingRow('h-release', 5, 2, 'Release'),
      headingRow('h-max', 6, 6, 'Max'),
      headingRow('h-back', 7, 3, 'Back'),
    ]
    const flags = async (flag: boolean | null) => {
      const tree = await commands.libraryGet(flag)
      return tree
        .filter(isHeading)
        .map((heading) => [heading.title, heading.level_skipped] as const)
    }
    // The flag set is the same under either scope (hidden instructions
    // are transparent to the pair walk).
    const expected = [
      ['Style', false],
      ['Tone', true],
      ['Deep', false],
      ['Release', false],
      ['Max', true],
      ['Back', false],
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
      { projectId: active!.id, instructionId: 'i-keeper', variantId: 'v-keeper' },
    ]
    const visible = await commands.libraryGet(null)
    expect(visible.map((entry) => entry.title)).toEqual(['Dropper'])
    const widened = await commands.libraryGet(true)
    const keeper = widened
      .filter(isInstruction)
      .find((instruction) => instruction.title === 'Keeper')
    expect(keeper?.state).toBe('hidden')
    expect(keeper?.project_labels.map((l) => l.project_id)).toEqual([active!.id])
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

  it('answers typed outcomes for registered and excluded paths and adds the rest', async () => {
    // The typed non-add outcomes mirror the real command: an active row is
    // `already_added` naming that row, an archived one `archived` with the
    // restore offer's id, an excluded path `excluded`; the discovery in the
    // same selection is still added — and re-sending the selection
    // converges (one row, one journal entry for the discovery).
    await scanWorld()
    const state = mockState()
    const known = state.projects.find((row) => row.state === 'active')
    const archived = state.projects.find((row) => row.state === 'archived')
    const excluded = state.exclusions[0]
    if (!known || !archived || !excluded) throw new Error('the fixture world must hold all three')
    const before = state.projects.length
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
    expect(byPath.get(excluded.path)).toEqual({
      path: excluded.path,
      outcome: 'excluded',
      project: null,
      existing: null,
    })
    expect(byPath.get(DISCOVERED)?.outcome).toBe('added')
    expect(byPath.get(known.path)).toMatchObject({
      outcome: 'already_added',
      project: null,
      existing: { id: known.id, state: 'active' },
    })
    expect(state.projects).toHaveLength(before + 1)
    expect(state.activity).toHaveLength(journalBefore + 1)

    const second = await commands.scanCommit([archived.path, excluded.path, DISCOVERED, known.path])
    expect(second.entries.map((entry) => entry.outcome)).toEqual([
      'archived',
      'excluded',
      'already_added',
      'already_added',
    ])
    expect(state.projects).toHaveLength(before + 1)
    expect(state.activity).toHaveLength(journalBefore + 1)
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
    await expect(commands.scanCommit([])).resolves.toEqual({ entries: [] })
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
    const second = await commands.variantCreate(instruction.id, 'Second', 'empty')
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
    const created = await commands.variantCreate(instruction.id, 'Second', 'empty')

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
        variantId: created.id,
      })
    }
    const before = state.assignments.filter((row) => row.instructionId === instruction.id).length

    const deleted = await commands.variantDelete(created.id, null)

    expect(deleted.state).toBe('deleted')
    expect(deleted.is_main).toBe(false)
    const after = state.assignments.filter((row) => row.instructionId === instruction.id)
    expect(after).toHaveLength(before)
    if (main) expect(after.every((row) => row.variantId === main.id)).toBe(true)
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
