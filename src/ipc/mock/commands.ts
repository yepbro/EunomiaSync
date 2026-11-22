// The mock command implementations (T-141): one object, annotated with the
// generated `IpcCommands` type, so the compiler enforces that EVERY command
// in `src/bindings.ts` has a mock with the exact signature — a regenerated
// bindings file with a new command fails `pnpm run typecheck` here until a
// mock is written (the same loud-drift rule the bindings tests pin on the
// Rust side; see DECISIONS.md D-154).
//
// Fidelity rules:
// - Read commands serve generator fixtures through the real wire DTOs.
// - Write commands mutate the in-memory store honestly: what a UI test
//   writes must be readable back through the corresponding read command
//   (the Settings screen's save-then-reread, the S1 add-then-list flow).
// - Error arms reject with the wire-shaped `AppError` (§7.10), never throw
//   synchronously.
//
// Known mock limits (deliberate, D-108): no filesystem or Git binary —
// paths are trusted strings, the `repo_root_offered` arm of
// `project_add_manual` never triggers, and its SEC-003 session-allowlist
// gate and §6.9 name-length validation are not modelled (an unpicked or
// absurdly named folder is accepted); the write-access probe always
// answers "writable"; `db_restore_backup`/`db_recreate` do not model the
// recovery-mode-only guard; `markdown_preview` is a small escaper with the
// one D-15 warning, not a pulldown-cmark mirror; D-29's tie-break is
// palette order, not an id hash.

import type {
  ActivityPageWire,
  ActivityEntryWire,
  AppInfo,
  ExclusionEntry,
  ExistingProject,
  IpcCommands,
  MarkdownPreview,
  ProjectAddResultWire,
  SettingChange,
} from '../contract'
import type { SyncRunOutcome_Serialize } from '../../bindings'
import { version as appVersion } from '../../../package.json'
import { defaultLabel, pathKeyOf } from '../fixtures/domain'
import { ACTION_MEMBERS, PALETTE } from '../fixtures/generator'
import { rejectWith } from './errors'
import {
  defaultsFromSettings,
  freshRowId,
  mockState,
  toAddedProject,
  toColumnEntry,
  type MockProjectRow,
} from './state'

/** The page size when the caller sends none (the real command's value). */
const DEFAULT_PAGE_LIMIT = 50
/** The largest page served (the real command's MAX_PAGE_LIMIT). */
const MAX_PAGE_LIMIT = 500

/**
 * The DB-002 UTC shape, character-level like `values::is_db002_shape`:
 * `YYYY-MM-DDTHH:MM:SS.sssZ` — digits at the non-punctuation positions,
 * calendar validity is NOT this check's job.
 */
function isDb002Shape(value: string): boolean {
  return /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/.test(value)
}

// ---------------------------------------------------------------------------
// Settings value shapes (§6.5 ranges): the mock's stand-in for the store's
// `SettingValue::from_json` — a change that cannot be a value of its key
// is an `E_VALIDATION` rejection, like the real boundary.
// ---------------------------------------------------------------------------

const isString = (v: unknown): v is string => typeof v === 'string'
const isBool = (v: unknown): v is boolean => typeof v === 'boolean'
const isIntBetween =
  (min: number, max: number) =>
  (v: unknown): boolean =>
    typeof v === 'number' && Number.isInteger(v) && v >= min && v <= max
// The store's `extra_file_from_json` shape: EXACTLY `enabled` (bool) and
// `position` (start|end) — any other field is a `SettingsError::Shape`
// rejection there, so the mock rejects it too, never silently drops it.
const isExtraHint = (v: unknown): boolean => {
  if (typeof v !== 'object' || v === null || Array.isArray(v)) return false
  const keys = Object.keys(v).sort()
  if (keys.length !== 2 || keys[0] !== 'enabled' || keys[1] !== 'position') return false
  const hint = v as { enabled?: unknown; position?: unknown }
  return isBool(hint.enabled) && (hint.position === 'start' || hint.position === 'end')
}

const SETTING_VALUE_SHAPES: Map<string, (v: unknown) => boolean> = new Map([
  ['default.scheme', (v) => v === 'A' || v === 'B' || v === 'C'],
  ['default.base_name', (v) => v === 'AGENTS.md' || v === 'CLAUDE.md'],
  ['default.extra_local', isExtraHint],
  ['default.extra_project', isExtraHint],
  ['backup.retention', isIntBetween(1, 200)],
  ['sync.skip_identical', isBool],
  ['scan.max_depth', isIntBetween(1, 20)],
  ['scan.exclude_names', (v) => Array.isArray(v) && v.every(isString)],
  ['history.max_revisions', isIntBetween(10, 1000)],
  ['log.max_entries', isIntBetween(1000, 100000)],
  ['log.max_days', isIntBetween(7, 3650)],
  ['ui.language', isString],
  ['update.check_on_start', isBool],
])

// ---------------------------------------------------------------------------
// Small helpers shared by the implementations
// ---------------------------------------------------------------------------

/** Appends one `action_log` row (LOG-001…004; ids increase per §6.2). */
function journal(
  action: string,
  fields: {
    level?: 'info' | 'warn' | 'error'
    visibility?: 'user' | 'diagnostic'
    result?: 'ok' | 'error' | 'skipped'
    entityType?: string | null
    entityId?: string | null
    projectId?: string | null
    details?: string | null
  } = {},
): void {
  const state = mockState()
  const entry: ActivityEntryWire = {
    id: state.nextActivityId++,
    ts: new Date().toISOString(),
    level: fields.level ?? 'info',
    visibility: fields.visibility ?? 'user',
    action,
    entity_type: fields.entityType ?? null,
    entity_id: fields.entityId ?? null,
    project_id: fields.projectId ?? null,
    sync_run_id: null,
    result: fields.result ?? 'ok',
    details: fields.details ?? null,
  }
  state.activity.push(entry)
}

/** D-29's rule: the least-used palette color among ACTIVE projects. */
function pickColor(projects: readonly MockProjectRow[]): string {
  const used = new Map<string, number>()
  for (const row of projects) {
    if (row.state !== 'active') continue
    used.set(row.color, (used.get(row.color) ?? 0) + 1)
  }
  let best = PALETTE[0] as string
  let bestCount = Infinity
  for (const color of PALETTE) {
    const count = used.get(color) ?? 0
    if (count < bestCount) {
      best = color
      bestCount = count
    }
  }
  // Tie-break: first palette member (the real command hashes the new id —
  // an approximation, documented in DECISIONS.md D-108's limits).
  return best
}

// ---------------------------------------------------------------------------
// markdown_preview: a small deterministic renderer with the SEC-005 policy
// (every raw-HTML fragment survives only as escaped visible text) and the
// one D-15 warning the spec names (the unclosed code fence). It is a
// stand-in for pulldown-cmark, not a mirror: paragraph/fence structure only.
// ---------------------------------------------------------------------------

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;')
}

function renderMarkdownPreview(text: string): MarkdownPreview {
  const lines = text.split(/\r?\n/)
  const html: string[] = []
  let paragraph: string[] = []
  let fenceOpenLine = 0
  let fenceLines: string[] = []
  let inFence = false
  const flushParagraph = (): void => {
    if (paragraph.length > 0) {
      html.push(`<p>${paragraph.map(escapeHtml).join('<br />')}</p>`)
      paragraph = []
    }
  }

  for (let index = 0; index < lines.length; index++) {
    const line = lines[index] as string
    if (line.trimStart().startsWith('```')) {
      if (inFence) {
        html.push(`<pre><code>${fenceLines.map(escapeHtml).join('\n')}</code></pre>`)
        fenceLines = []
        inFence = false
      } else {
        flushParagraph()
        inFence = true
        fenceOpenLine = index + 1
      }
      continue
    }
    if (inFence) {
      fenceLines.push(line)
    } else if (line.trim() === '') {
      flushParagraph()
    } else {
      paragraph.push(line)
    }
  }
  flushParagraph()

  const warnings: MarkdownPreview['warnings'] = []
  if (inFence) {
    // The author's bug is the missing closing fence: everything after the
    // opening line renders as code, which is exactly what we emit.
    html.push(`<pre><code>${fenceLines.map(escapeHtml).join('\n')}</code></pre>`)
    warnings.push({ kind: 'unclosed_code_fence' as const, line: fenceOpenLine })
  }
  return { html: html.join('\n'), warnings }
}

// ---------------------------------------------------------------------------
// The command surface
// ---------------------------------------------------------------------------

export const mockCommands: IpcCommands = {
  /** `projects_list` (§7.10): column order is `sort_order` (ALG-001). */
  async projectsList(includeArchived) {
    const rows = [...mockState().projects].sort((a, b) => a.sortOrder - b.sortOrder)
    const visible = includeArchived === true ? rows : rows.filter((p) => p.state === 'active')
    return visible.map(toColumnEntry)
  },

  /**
   * `projects_reorder` (§7.10, T-080): validates exactly like the real
   * command — the list must name every active project exactly once each
   * (unknown/archived ids, missing actives and repeats are the typed
   * `E_VALIDATION` refusal naming the counts) — then renumbers the table
   * `1..=N` with the archived rows keeping their own slots (D-093),
   * journals one `project.changed` entry carrying the moved count, and
   * answers a re-sent unchanged order as a no-op (nothing journaled).
   */
  async projectsReorder(orderedIds) {
    const state = mockState()
    const rows = [...state.projects].sort((a, b) => a.sortOrder - b.sortOrder)
    const activeIds = rows.filter((p) => p.state === 'active').map((p) => p.id)
    const seen = new Set<string>()
    let duplicates = 0
    for (const id of orderedIds) {
      if (seen.has(id)) duplicates++
      else seen.add(id)
    }
    const unknown = orderedIds.filter((id) => !activeIds.includes(id))
    const missing = activeIds.filter((id) => !seen.has(id)).length
    if (unknown.length > 0 || missing > 0 || duplicates > 0) {
      // The value mirrors the real envelope exactly: the offending
      // caller ids capped at five ids and 64 code points each, or —
      // when nothing offends — what the caller did send (D-115).
      const ECHO_LIMIT = 5
      const ID_CHARS = 64
      const bounded = (id: string): string => Array.from(id).slice(0, ID_CHARS).join('')
      const value =
        unknown.length === 0
          ? `${orderedIds.length} ids sent`
          : unknown.length <= ECHO_LIMIT
            ? unknown.map(bounded).join(', ')
            : `${unknown.slice(0, ECHO_LIMIT).map(bounded).join(', ')} (+${unknown.length - ECHO_LIMIT} more)`
      return rejectWith('E_VALIDATION', 'validation.invalid_value', {
        field: 'ordered_ids',
        value,
        reason: `the list must name every active project exactly once: ${unknown.length} not an active project, ${missing} missing, ${duplicates} repeated`,
      })
    }
    // The composition (D-093): the active slots take the input ids in
    // sequence, the archived rows keep their own slots.
    const input = [...orderedIds]
    const reordered = rows.map((p) => (p.state === 'active' ? (input.shift() as string) : p.id))
    const changed = rows.filter((p, index) => p.id !== reordered[index]).length
    if (changed === 0) return null
    let position = 0
    for (const id of reordered) {
      position++
      const row = state.projects.find((p) => p.id === id)
      if (row) row.sortOrder = position
    }
    journal('project.changed', { details: JSON.stringify({ changed }) })
    return null
  },

  /**
   * `project_add_manual` (§7.10/S1): the duplicate arms by normalized path,
   * the add with fields derived exactly like the Rust command (name from
   * the folder, DB-042 label, D-29 color, appended `sort_order`, write
   * settings copied from `default.*`), and the `project.added` journal
   * entry. The `repo_root_offered` arm needs a filesystem and never
   * triggers in the mock.
   */
  async projectAddManual(path, acceptRepoRoot) {
    // Referenced to keep the wire seam visible: the offer flow is the real
    // command's, pinned by its Rust tests; the mock accepts the flag only.
    void acceptRepoRoot
    const trimmed = path.trim()
    if (trimmed === '') {
      return rejectWith('E_VALIDATION', 'validation.path_required', { field: 'path' })
    }
    const state = mockState()
    const key = pathKeyOf(trimmed)
    const existingRow = state.projects.find((row) => pathKeyOf(row.path) === key)
    if (existingRow) {
      const existing: ExistingProject = {
        id: existingRow.id,
        name: existingRow.name,
        path: existingRow.path,
        state: existingRow.state,
      }
      const outcome = existingRow.state === 'active' ? 'duplicate_active' : 'archived_duplicate'
      const result: ProjectAddResultWire = { outcome, project: null, offered_root: null, existing }
      return result
    }
    const defaults = defaultsFromSettings(state.settings)
    const segments = key.split('/')
    const name = (segments[segments.length - 1] ?? '').trim() || 'project'
    const row: MockProjectRow = {
      id: freshRowId(),
      path: key,
      name,
      label: defaultLabel(name),
      color: pickColor(state.projects),
      state: 'active',
      addedVia: 'manual',
      sortOrder: state.projects.reduce((max, p) => Math.max(max, p.sortOrder), 0) + 1,
      scheme: defaults.scheme,
      baseName: defaults.baseName,
      extraLocalEnabled: defaults.extraLocalEnabled,
      extraLocalPosition: defaults.extraLocalPosition,
      extraProjectEnabled: defaults.extraProjectEnabled,
      extraProjectPosition: defaults.extraProjectPosition,
      available: true,
      unavailableReason: null,
      dirty: false,
      // §5.7's clean baseline for a fresh row: never synced, no file
      // verdict exists (OUT-005: the sync creates the file) — the
      // watch-service mock twin would compute exactly this.
      externalChanged: false,
      fileDeleted: false,
      lastSyncedAt: null,
    }
    state.projects.push(row)
    journal('project.added', { entityType: 'project', entityId: row.id, projectId: row.id })
    return { outcome: 'added', project: toAddedProject(row), offered_root: null, existing: null }
  },

  /** `settings_get` (§7.10): the current map in §6.5 table order. */
  async settingsGet() {
    return mockState().settings.map((s) => ({ ...s }))
  },

  /**
   * `settings_set` (§7.10/§6.9): all-or-nothing — every change validates
   * first (known key, JSON text, a value of the key's shape); only then do
   * the values apply and one `settings.changed` entry per key join the
   * journal. The response is the fresh full map.
   */
  async settingsSet(changes) {
    const state = mockState()
    const parsed: { change: SettingChange; canonical: string }[] = []
    for (const change of changes) {
      const shape = SETTING_VALUE_SHAPES.get(change.key)
      if (!shape) {
        return rejectWith('E_VALIDATION', 'validation.unknown_setting', { key: change.key })
      }
      let value: unknown
      try {
        value = JSON.parse(change.value)
      } catch {
        return rejectWith('E_VALIDATION', 'validation.invalid_setting_json', {
          key: change.key,
        })
      }
      if (!shape(value)) {
        return rejectWith('E_VALIDATION', 'validation.invalid_setting_value', {
          key: change.key,
        })
      }
      // Store the CANONICAL form (the `SettingItem.value` contract: "the
      // exact form stored and re-read") — not the caller's spelling: the
      // extra hints re-serialize in struct order (`enabled`, `position`).
      const canonical =
        change.key === 'default.extra_local' || change.key === 'default.extra_project'
          ? JSON.stringify({
              enabled: (value as { enabled: boolean }).enabled,
              position: (value as { position: string }).position,
            })
          : JSON.stringify(value)
      parsed.push({ change, canonical })
    }
    for (const { change, canonical } of parsed) {
      const target = state.settings.find((s) => s.key === change.key)
      if (target) target.value = canonical
      journal('settings.changed', { entityType: 'setting', entityId: change.key })
    }
    return state.settings.map((s) => ({ ...s }))
  },

  /**
   * `activity_list` (§7.10/§7.7): the boundary validates exactly like the
   * real command (`validated_activity_query`): limit 1..=500 (default 50),
   * offset ≥ 0, DB-002-shaped date bounds, a non-inverted range, actions
   * within the closed 22-member vocabulary, result within {ok, error,
   * skipped} — every rejection is `E_VALIDATION` naming field/value/reason.
   * Then: every filter combines with AND; LOG-005 keeps diagnostics out
   * unless `include_diagnostic` explicitly opts in; newest first (`ts`
   * desc, `id` desc); `total` counts the filters' matches regardless of
   * paging.
   */
  async activityList(query) {
    const limit = query.limit ?? DEFAULT_PAGE_LIMIT
    if (!Number.isInteger(limit) || limit < 1 || limit > MAX_PAGE_LIMIT) {
      return rejectWith('E_VALIDATION', 'validation.invalid_value', {
        field: 'limit',
        value: String(limit),
        reason: `must be between 1 and ${MAX_PAGE_LIMIT}`,
      })
    }
    const offset = query.offset ?? 0
    if (!Number.isInteger(offset) || offset < 0) {
      return rejectWith('E_VALIDATION', 'validation.invalid_value', {
        field: 'offset',
        value: String(offset),
        reason: 'must be 0 or greater',
      })
    }
    for (const field of ['from_ts', 'to_ts'] as const) {
      const bound = query[field]
      if (bound !== null && bound !== undefined && !isDb002Shape(bound)) {
        return rejectWith('E_VALIDATION', 'validation.invalid_value', {
          field,
          value: bound,
          reason: 'must be a DB-002 UTC timestamp (YYYY-MM-DDTHH:MM:SS.sssZ)',
        })
      }
    }
    const from = query.from_ts ?? null
    const to = query.to_ts ?? null
    if (from !== null && to !== null && from > to) {
      return rejectWith('E_VALIDATION', 'validation.invalid_value', {
        field: 'from_ts',
        value: from,
        reason: 'the range is inverted: from_ts is later than to_ts',
      })
    }
    const actions = query.actions ?? null
    if (actions !== null) {
      for (const action of actions) {
        if (!ACTION_MEMBERS.includes(action)) {
          return rejectWith('E_VALIDATION', 'validation.invalid_value', {
            field: 'actions',
            value: action,
            reason: 'unknown action',
          })
        }
      }
    }
    if (query.result !== null && query.result !== undefined) {
      if (query.result !== 'ok' && query.result !== 'error' && query.result !== 'skipped') {
        return rejectWith('E_VALIDATION', 'validation.invalid_value', {
          field: 'result',
          value: query.result,
          reason: 'must be ok, error or skipped',
        })
      }
    }
    const filtered = mockState()
      .activity.filter((entry) => {
        if (entry.visibility === 'diagnostic' && query.include_diagnostic !== true) {
          return false
        }
        if (from !== null && entry.ts < from) return false
        if (to !== null && entry.ts > to) return false
        if (query.project_id !== null && query.project_id !== undefined) {
          if (entry.project_id !== query.project_id) return false
        }
        if (actions !== null && actions.length > 0 && !actions.includes(entry.action)) {
          return false
        }
        if (query.result !== null && query.result !== undefined && entry.result !== query.result) {
          return false
        }
        return true
      })
      .sort((a, b) => (a.ts === b.ts ? b.id - a.id : a.ts < b.ts ? 1 : -1))
    const page: ActivityPageWire = {
      entries: filtered.slice(offset, offset + limit),
      total: filtered.length,
    }
    return page
  },

  /** `app_info` (§7.10): the About window's facts, canned (T-079's DTO). */
  async appInfo(): Promise<AppInfo> {
    return {
      // package.json's version (the real source is CARGO_PKG_VERSION,
      // which tauri.conf.json mirrors — they move together).
      version: appVersion,
      identifier: 'pro.yepbro.eunomiasync',
      author: 'Den Brooks <denis@yepbro.pro>',
      license: 'MIT OR Apache-2.0',
      repository: 'https://github.com/yepbro/EunomiaSync',
      data_dir: '/home/dev/.local/share/eunomiasync',
      db_created_fresh: false,
      running_as_root: false,
    }
  },

  /** `app_open_data_folder` (§7.10): no OS here — a resolved no-op. */
  async appOpenDataFolder() {
    return null
  },

  /** `db_backups_list` (§7.10/§10): copies newest first. */
  async dbBackupsList() {
    return [...mockState().backups].sort((a, b) => (a.modified_utc < b.modified_utc ? 1 : -1))
  },

  /**
   * `db_restore_backup` (§7.10/§10): a known copy resolves (the mock has
   * nothing to reopen, so the store is untouched); an unknown name is the
   * boundary's `E_VALIDATION`.
   */
  async dbRestoreBackup(backupName) {
    const known = mockState().backups.some((b) => b.name === backupName)
    if (!known) {
      return rejectWith('E_VALIDATION', 'validation.unknown_backup', {
        backup_name: backupName,
      })
    }
    return null
  },

  /** `db_recreate` (§7.10/§10): resolves; the mock store stays as-is. */
  async dbRecreate() {
    return null
  },

  /**
   * `exclusions_list` (§7.10 Projects module, the D-10 screen): the
   * store's own stable order — `created_at`, the UUIDv7 id second: oldest
   * first, id order inside one millisecond. A fresh store answers an
   * empty list (the screen's empty state), never null.
   */
  async exclusionsList(): Promise<ExclusionEntry[]> {
    return [...mockState().exclusions].sort((a, b) =>
      a.created_at === b.created_at ? (a.id < b.id ? -1 : 1) : a.created_at < b.created_at ? -1 : 1,
    )
  },

  /**
   * `exclusion_remove` (§7.10, T-078/D-106): id-addressed only — no path
   * input (SEC-003's spirit); a stale id is the typed `E_VALIDATION`
   * refusal naming the field, with nothing deleted and nothing journaled;
   * a hit removes the row (the next `exclusions_list` no longer serves
   * it) and journals the removal in one operation. The journal entry
   * mirrors the real command exactly (the D-106 follow-up member,
   * D-119): `exclusion.removed` at info/user/ok whose entity columns
   * name the removed row (`exclusion` + its id) and whose `details`
   * carry the removed path as `{"path":"…"}`; no `project_id` (no
   * project row is involved).
   */
  async exclusionRemove(id) {
    const state = mockState()
    const index = state.exclusions.findIndex((row) => row.id === id)
    if (index === -1) {
      return rejectWith('E_VALIDATION', 'validation.invalid_value', {
        field: 'id',
        value: id,
        reason: 'no exclusion with this id',
      })
    }
    const [removed] = state.exclusions.splice(index, 1)
    journal('exclusion.removed', {
      entityType: 'exclusion',
      entityId: id,
      details: JSON.stringify({ path: removed?.path }),
    })
    return null
  },

  /**
   * `dialog_pick_folder` (§7.10, SEC-001/SEC-003): the picked folder the
   * later `project_add_manual` accepts. Suffixing with the store size lets
   * one dev session run the S1 flow repeatedly without hitting the
   * duplicate arm; `null` (the cancel answer) is modeled by editing the
   * state or by the future test seam.
   */
  async dialogPickFolder() {
    return `/home/dev/repos/picked-project-${mockState().projects.length}`
  },

  /** `markdown_preview` (§7.10, SEC-005/D-15): sanitized HTML + warnings. */
  async markdownPreview(text) {
    return renderMarkdownPreview(text)
  },

  /**
   * `sync_run` (§7.10, §9.4; T-118): a stand-in over the mock state,
   * not a pipeline mirror (D-108's limits posture): no library or
   * assignment state exists here, so no build is possible — every
   * AVAILABLE active project is `written` (its `dirty` flag and both
   * §5.7 watch markings clear and `lastSyncedAt` stamps, readable
   * back through `projectsList` — the mock twin of the backend's
   * after-sync pass), every UNAVAILABLE one skips with `E_PROJECT_UNAVAILABLE` (D-09:
   * skipped, never stopping the run), and the run journals one
   * `sync.project_result` per project plus the `sync.run_finished`
   * summary (the helper's `sync_run_id` stays null — the mock journal
   * has no run link, a documented approximation). The D-25
   * `needsConfirmation` arm and the `E_SYNC_RUNNING` refusal are not
   * modelled (no builds, no flag state): the `confirmEmpty` argument
   * is accepted and ignored (hence no named parameter).
   */
  async syncRun(): Promise<SyncRunOutcome_Serialize> {
    const state = mockState()
    const rows = [...state.projects]
      .filter((row) => row.state === 'active')
      .sort((a, b) => a.sortOrder - b.sortOrder)
    const now = new Date().toISOString()
    const runId = freshRowId()
    const projects = rows.map((row) => {
      const fileName =
        row.scheme === 'C' ? row.baseName.replace(/\.md$/, '.project.md') : row.baseName
      const target = `${row.path.replace(/\/$/, '')}/${fileName}`
      if (!row.available) {
        journal('sync.project_result', {
          level: 'warn',
          result: 'skipped',
          projectId: row.id,
        })
        return {
          project_id: row.id,
          status: 'skipped',
          target_path: target,
          error_code: 'E_PROJECT_UNAVAILABLE',
          error_message: 'the mock row is marked unavailable',
        }
      }
      // The after-sync row update, mock twin of the backend's
      // post-sync pass (D-137/D-144): a written file matches its
      // build again, so the §5.7 markings clear with `dirty` — the
      // real `state://file-changed` diff would carry the clearing.
      row.dirty = false
      row.externalChanged = false
      row.fileDeleted = false
      row.lastSyncedAt = now
      journal('sync.project_result', { projectId: row.id })
      return {
        project_id: row.id,
        status: 'written',
        target_path: target,
        error_code: null,
        error_message: null,
      }
    })
    const written = projects.filter((entry) => entry.status === 'written').length
    const skipped = projects.filter((entry) => entry.status === 'skipped').length
    const verdict = skipped === 0 ? 'success' : written > 0 ? 'partial' : 'failed'
    journal('sync.run_finished', {
      level: verdict === 'success' ? 'info' : 'warn',
      result: verdict === 'success' ? 'ok' : 'skipped',
    })
    return {
      outcome: 'completed',
      summary: {
        run_id: runId,
        verdict,
        started_at: now,
        finished_at: now,
        projects_total: projects.length,
        projects_written: written,
        projects_unchanged: 0,
        projects_skipped: skipped,
        projects_failed: 0,
        details: projects.length === 0 ? 'no active projects to synchronize' : null,
        projects,
      },
    }
  },
}

export { SETTING_VALUE_SHAPES }
