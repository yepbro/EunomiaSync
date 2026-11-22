import { beforeEach, describe, expect, it } from 'vitest'
import { mockCommands } from './commands'
import { mockState, resetMockState } from './state'
import { mockEvents } from './events'
import type { FixtureProfile } from '../fixtures/generator'

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
    const archived = (await commands.projectsList(true)).find((p) => p.state === 'archived')
    expect(archived?.external_changed).toBe(false)
    expect(archived?.file_deleted).toBe(false)
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
    // twin — `scan_commit` is T-077's).
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
