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
// answers "writable"; `scan_commit` models the discovered-set rule (only
// paths the last completed `scanStart` reported are admitted) but none of
// its filesystem gates (a reported path that vanished or lost its `.git`
// since the scan is added here); `db_restore_backup`/`db_recreate` do not
// model the recovery-mode-only guard; `markdown_preview` is a small
// escaper with the one D-15 warning, not a pulldown-cmark mirror; D-29's
// tie-break is palette order, not an id hash.

import type {
  ActivityPageWire,
  ActivityEntryWire,
  AssignmentTargetWire,
  BulkAssignmentResultWire,
  AppInfo,
  ExclusionEntry,
  ExistingProject,
  HeadingRecordWire,
  HeadingUpdate,
  IpcCommands,
  MarkdownPreview,
  MoveTarget,
  ProjectAddResultWire,
  ProjectLabelEntry,
  RevisionRecordWire,
  ScanCommitEntryWire,
  ScanRunOutcomeWire,
  ScannedRepository,
  SettingChange,
  SettingItem,
  ScanSkipWire,
} from '../contract'
import type { SyncRunOutcome_Serialize } from '../../bindings'
import { version as appVersion } from '../../../package.json'
import { defaultLabel, pathKeyOf } from '../fixtures/domain'
import { ACTION_MEMBERS, PALETTE } from '../fixtures/generator'
import { rejectWith } from './errors'
import { mockEvents } from './events'
import {
  defaultsFromSettings,
  freshRowId,
  mockState,
  toAddedProject,
  toAssignmentRecord,
  toBackupCopyRecord,
  toColumnEntry,
  toVariantRecord,
  type MockAssignmentRow,
  type MockLibraryRow,
  type MockProjectRow,
  type MockState,
  type MockVariantRow,
} from './state'

/** The page size when the caller sends none (the real command's value). */
const DEFAULT_PAGE_LIMIT = 50
/** The largest page served (the real command's MAX_PAGE_LIMIT). */
const MAX_PAGE_LIMIT = 500

/**
 * The DB-006 text ceiling (§6.9), the mock's copy of the real limit —
 * the boundary test's other edge.
 */
const VARIANT_TEXT_LIMIT = 262_144

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
// The variant commands' shared helpers (§7.10 Library module, T-329): the
// mock twins of the store's own queries and the Rust command's §6.9 rules,
// over the union-shaped library rows (T-083's MockLibraryRow). The D-19
// marker literals, the built-form splitter and the ASCII NOCASE fold are
// the shared helpers below (T-328's landing); the name verdict, the trim
// class and the save-time fold are this group's own.
// ---------------------------------------------------------------------------

/** The `E_VALIDATION` `variant_name` field refusal (the Rust command's
 * `field_refusal` shape: field + reason, never the name itself). */
function rejectVariantName(reason: string): Promise<never> {
  return rejectWith('E_VALIDATION', 'errors.validation', {
    field: 'variant_name',
    reason,
  })
}

/** The name's first rule offense by POSITION (the validator's own
 * `name_problem` order): one scan reporting whichever of NUL or a line
 * break comes first, with its code-point index and the offending
 * character's stable name — the mock twin of
 `LibraryFieldError::VariantNameLineBreak`/`VariantNameNul`'s shared
 * scan, so `"a\0b\nc"` answers NUL at index 1 like the backend, not the
 * break at index 3. */
function firstNameOffense(trimmed: string): {
  index: number
  kind: 'break' | 'nul'
  character: string
} | null {
  const characters = Array.from(trimmed)
  for (let index = 0; index < characters.length; index++) {
    const character = characters[index] as string
    if (character === '\0') return { index, kind: 'nul', character: 'NUL' }
    if (character === '\n' || character === '\r') {
      return { index, kind: 'break', character: character === '\n' ? 'LF' : 'CR' }
    }
  }
  return null
}

/** Rust `str::trim()`'s character class, not JavaScript's: the Unicode
 * `White_Space` set — which includes U+0085 (JS `trim` keeps it) and
 * excludes the BOM U+FEFF (JS `trim` removes it). The mock's names must
 * trim exactly like the validator's, or padded-legal inputs persist
 * differently on the two sides. */
const RUST_TRIM =
  '[\\t\\n\\u000B\\u000C\\r \\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]'
const RUST_TRIM_RE = new RegExp(`^${RUST_TRIM}+|${RUST_TRIM}+$`, 'g')

/** `str::trim()`'s exact behavior, for the mock's §6.9 name rules. */
function rustTrim(value: string): string {
  return value.replace(RUST_TRIM_RE, '')
}

/** The §6.9 name rules over a TRIMMED name (the validator's own
 * order: the length bounds first, then the one positional scan) —
 * answers the rejection, or null when the name is legal. */
function nameRejection(name: string): Promise<never> | null {
  const trimmed = rustTrim(name)
  if (trimmed === '') {
    return rejectVariantName('a variant name needs 1-100 characters after trimming')
  }
  const codePoints = Array.from(trimmed).length
  if (codePoints > 100) {
    return rejectWith('E_VALIDATION', 'errors.validation', {
      field: 'variant_name',
      reason: 'a variant name is at most 100 characters',
      length: String(codePoints),
    })
  }
  const offense = firstNameOffense(trimmed)
  if (offense) {
    return rejectWith('E_VALIDATION', 'errors.validation', {
      field: 'variant_name',
      reason:
        offense.kind === 'nul'
          ? 'a variant name must not contain NUL'
          : 'a variant name is one line',
      index: String(offense.index),
      character: offense.character,
    })
  }
  return null
}

/** The instruction row the variant commands address (the union's
 * kind-narrowed lookup — the `instructionRowOf` twin for the variant
 * group's own refusal shape), or null when no such row exists. */
function variantInstructionOf(
  state: MockState,
  id: string,
): (MockLibraryRow & { kind: 'instruction' }) | null {
  const row = state.library.find((item) => item.id === id)
  return row && row.kind === 'instruction' ? row : null
}

/** The variant row with its owning instruction (the by-id lookup the
 * four id-addressed twins share), or null when no instruction holds a
 * variant with this id. */
function findVariantRow(
  state: MockState,
  id: string,
): { instruction: MockLibraryRow & { kind: 'instruction' }; variant: MockVariantRow } | null {
  for (const row of state.library) {
    if (row.kind !== 'instruction') continue
    const variant = row.variants.find((candidate) => candidate.id === id)
    if (variant) return { instruction: row, variant }
  }
  return null
}

/** The instruction's ACTIVE main variant (DB-010's exactly-one). */
function activeMainOf(variants: readonly MockVariantRow[]) {
  return variants.find((row) => row.isMain && row.state === 'active') ?? null
}

/** Whether an ACTIVE variant of the instruction already carries the
 * name NOCASE-equal (`exceptId` exempts the row being renamed) — the
 * shared `asciiFold`, SQLite's own ASCII-only fold. */
function nameTaken(
  variants: readonly MockVariantRow[],
  name: string,
  exceptId: string | null,
): boolean {
  const wanted = asciiFold(name)
  return variants.some(
    (row) => row.state === 'active' && row.id !== exceptId && asciiFold(row.name) === wanted,
  )
}

/** §6.9's save-time CRLF→LF fold (the store's own clause, D-076). */
function foldCrlf(text: string): string {
  return text.replace(/\r\n/g, '\n')
}

/** The custom-text field's §6.9 verdict, with no text echo. */
function customTextRejection(text: string): Promise<never> | null {
  const verdict = variantTextRejection(text)
  if (!verdict) return null
  if (verdict.kind === 'marker') {
    return rejectWith('E_CONTENT_RESERVED', 'errors.content_reserved', {
      field: 'custom_text',
      line: String(verdict.line),
      marker: verdict.marker,
    })
  }
  const characters = Array.from(text)
  return rejectWith('E_VALIDATION', 'errors.validation', {
    field: 'custom_text',
    reason:
      verdict.kind === 'length'
        ? `custom text is at most ${VARIANT_TEXT_LIMIT} characters`
        : 'custom text must not contain NUL',
    ...(verdict.kind === 'length'
      ? { length: String(characters.length) }
      : { index: String(characters.indexOf('\0')) }),
  })
}

/** SHA-256 of the canonical snapshot, through the platform Web Crypto API. */
async function mockSnapshotHash(snapshot: string): Promise<string> {
  const digest = await globalThis.crypto.subtle.digest(
    'SHA-256',
    new TextEncoder().encode(snapshot),
  )
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, '0')).join('')
}

/** Records one real library change and applies the per-entity history limit. */
async function recordMockRevision(
  entityType: 'instruction' | 'variant',
  entityId: string,
  op: RevisionRecordWire['op'],
  snapshot: string,
): Promise<void> {
  const state = mockState()
  const revisions = state.revisions.filter(
    (row) => row.entity_type === entityType && row.entity_id === entityId,
  )
  const revNo = Math.max(0, ...revisions.map((row) => Number(row.rev_no))) + 1
  state.revisions.push({
    id: freshRowId(),
    entity_type: entityType,
    entity_id: entityId,
    rev_no: String(revNo),
    op,
    snapshot,
    snapshot_hash: await mockSnapshotHash(snapshot),
    reverted_from: null,
    created_at: new Date().toISOString(),
  })
  const configured = state.settings.find((row) => row.key === 'history.max_revisions')?.value
  const limit = Math.max(1, Number(JSON.parse(configured ?? '200')))
  const kept = state.revisions
    .filter((row) => row.entity_type === entityType && row.entity_id === entityId)
    .sort((left, right) => Number(right.rev_no) - Number(left.rev_no))
    .slice(0, limit)
    .map((row) => row.id)
  state.revisions = state.revisions.filter(
    (row) => row.entity_type !== entityType || row.entity_id !== entityId || kept.includes(row.id),
  )
}

function assignmentOf(state: MockState, projectId: string, instructionId: string) {
  return state.assignments.find(
    (row) => row.projectId === projectId && row.instructionId === instructionId,
  )
}

function journalAssignment(
  projectId: string | null,
  instructionId: string | null,
  changed: number,
): void {
  journal('assignment.changed', {
    entityType: 'assignment',
    entityId: projectId === null || instructionId === null ? null : `${projectId}/${instructionId}`,
    projectId,
    details: JSON.stringify({ changed }),
  })
}

/** The first reserved marker line (DB-016) in the text as the build
 * writes it — over the shared `splitBuiltForm` (the CRLF/lone-CR fold
 * and the leading-edge drop), with the real scan's own trailing
 * ASCII-whitespace trim first (`char::is_ascii_whitespace`, not JS
 * `\s`), each line then equal to a D-19 literal modulo ASCII space/tab
 * padding, at the built form's own 1-based line number. */
function findReservedLine(text: string): { line: number; marker: string } | null {
  // The ASCII-whitespace trailing trim, the short-escape spelling the
  // shared `OUTPUTS_NO_TEXT` regex below uses (the `no-control-regex`
  // rule tolerates `\v`/`\f`, not their `\u`-escape spellings).
  const trailingTrimmed = text.replace(/[\t\n\v\f\r ]+$/, '')
  const lines = splitBuiltForm(trailingTrimmed)
  for (let index = 0; index < lines.length; index++) {
    const line = (lines[index] ?? '').replace(/^[ \t]+|[ \t]+$/g, '')
    if (line === BEGIN_MARKER) return { line: index + 1, marker: BEGIN_MARKER }
    if (line === END_MARKER) return { line: index + 1, marker: END_MARKER }
  }
  return null
}

/**
 * Appends one project row the way BOTH add commands register one — the
 * Rust side's shared create step (D-179): the name from the folder, the
 * DB-042 label, the D-29 color over the ACTIVE rows, the appended
 * `sort_order`, the write settings copied from `default.*`, the clean §5.7
 * baseline, and the `project.added` journal entry. `key` is the canonical
 * path (the mock has no filesystem: the key IS the path).
 */
function appendProjectRow(
  state: MockState,
  key: string,
  addedVia: MockProjectRow['addedVia'],
): MockProjectRow {
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
    addedVia,
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
    // And no lost watch: the file was never observed, so nothing
    // was lost from observation (the notice's cold baseline,
    // D-155).
    unwatchedReason: null,
    lastSyncedAt: null,
  }
  state.projects.push(row)
  journal('project.added', { entityType: 'project', entityId: row.id, projectId: row.id })
  return row
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
// The four instruction commands (T-328, §7.10 Library module, D-188):
// the mock twins of the Rust family — the same §6.9 validation order
// (name, then text), the same typed envelopes (DB-016's marker line is
// E_CONTENT_RESERVED, its own §10 row), the same D-188 rules (the
// section placement, the same-name no-op, the hide/restore
// idempotence, the class-only duplicate arm on restore). Known mock
// limits, deliberate (D-108's posture): no `revisions` rows (the mock
// store has no version table — the version DoDs are the Rust suite's),
// no assignment data to keep (the assignments are not wired into the
// mock store yet; the real commands do not touch them either, which
// the mock mirrors by not touching what does not exist), and the
// marker scan is a line `trim()` equality over the CRLF/BOM-aware
// split — the `classify_line` mirror at the shapes tests send.
// ---------------------------------------------------------------------------

/** The D-19 marker literals (the Rust `markers` module's twins). */
const BEGIN_MARKER = '<!-- EUNOMIASYNC:BEGIN -->'
const END_MARKER = '<!-- EUNOMIASYNC:END -->'

/** The D-115 bounded echo, the real `bounded_echo`'s twin: caller
 * input echoed into a rejection envelope is capped at 64 code points,
 * with the truncation named. */
function boundedEcho(value: string): string {
  const characters = Array.from(value)
  if (characters.length <= 64) return value
  return `${characters.slice(0, 64).join('')} (+${characters.length - 64} more code points)`
}

/** The §6.9 name verdict: the trimmed form, or the rejection reason —
 * the real `validate_instruction_title`'s mirror (trim-then-count, one
 * line, the D-126 name-wide NUL rule), with the same class-only
 * reason texts. */
function validateInstructionName(name: string): { trimmed: string } | { reason: string } {
  const trimmed = name.trim()
  const characters = Array.from(trimmed)
  if (characters.length === 0) {
    return { reason: 'instruction name is empty after trimming' }
  }
  if (characters.length > 200) {
    return {
      reason: `instruction name is ${characters.length} characters after trimming; the limit is 200`,
    }
  }
  const offending = characters.findIndex((ch) => ch === '\n' || ch === '\r' || ch === '\0')
  if (offending >= 0) {
    const character = characters[offending]
    return character === '\0'
      ? { reason: `instruction name contains NUL at index ${offending}` }
      : {
          reason: `instruction name contains the line break ${JSON.stringify(character)} at index ${offending}`,
        }
  }
  return { trimmed }
}

/** The §6.9 variant-text verdict (`null` when legal): DB-006's length
 * ceiling, the NUL rejection, and DB-016's reserved marker line — a
 * line equal to a D-19 literal modulo padding (the real check runs
 * over the ALG-002-normalized form; the split here handles CRLF and a
 * leading BOM is stripped, the shapes a test sends). The marker arm
 * carries which literal and the 1-based line number, the real
 * envelope's own facts. */
function variantTextRejection(
  text: string,
):
  | { kind: 'length' | 'nul'; reason: string }
  | { kind: 'marker'; marker: string; line: number }
  | null {
  const characters = Array.from(text)
  if (characters.length > 262_144) {
    return {
      kind: 'length',
      reason: `variant text is ${characters.length} characters; the limit is 262144`,
    }
  }
  const nul = characters.indexOf('\0')
  if (nul >= 0) {
    return { kind: 'nul', reason: `variant text contains NUL at index ${nul}` }
  }
  // The scan mirrors the real path's BUILT form as far as a
  // line-based mock can: the CRLF/CR-aware split, the leading edge —
  // the BOM run and empty lines dropped IN INTERLEAVED fixed-point
  // passes, exactly `drop_leading_edge`'s loop (a BOM after a blank
  // line is still leading; the LINE NUMBER the real envelope carries
  // counts this form) — and each line equal to a D-19 literal modulo
  // ASCII spaces and tabs (`classify_line`'s own padding — Unicode
  // whitespace padding does NOT match here, exactly as it does not
  // there).
  const lines = splitBuiltForm(text)
  for (let index = 0; index < lines.length; index++) {
    const line = (lines[index] ?? '').replace(/^[ \t]+|[ \t]+$/g, '')
    if (line === BEGIN_MARKER || line === END_MARKER) {
      return { kind: 'marker', marker: line, line: index + 1 }
    }
  }
  return null
}

/**
 * The built form's line list, the `variant_text` + `classify_line`
 * composition's mock twin: line breaks normalized (CRLF and a lone
 * CR both become LF — the split accepts every spelling), then the
 * leading edge dropped to its fixed point: the BOM run stripped and
 * empty (ASCII whitespace only) lines removed, interleaved, until
 * neither consumes anything (a BOM that becomes leading only after a
 * blank line drops is still leading — `drop_leading_edge`'s own
 * rule).
 */
function splitBuiltForm(text: string): string[] {
  const lines = text.split(/\r\n|\r|\n/)
  let start = 0
  let changed = true
  while (changed) {
    changed = false
    while ((lines[start] ?? '').startsWith('\uFEFF')) {
      lines[start] = (lines[start] ?? '').replace(/^\uFEFF+/, '')
      changed = true
    }
    while (start < lines.length && /^[ \t]*$/.test(lines[start] ?? '')) {
      start++
      changed = true
    }
  }
  return lines.slice(start)
}

/** The D-188 section rule's target position (the real
 * `section_target`'s twin, over the position-ordered rows): the end of
 * the picked heading's ALG-060 range — the first heading of level
 * `<=` its own that follows it, or the table end — or, for the root
 * pick (`null`), the end of the prefix before the FIRST heading. A
 * `section` that names no row or an instruction answers the refusal
 * reason instead (the typed field rejection's input). */
function sectionTarget(
  ordered: readonly MockLibraryRow[],
  section: string | null,
): number | { refusal: string } {
  if (section === null) {
    const firstHeading = ordered.find((row) => row.kind === 'heading')
    return firstHeading ? firstHeading.position : ordered.length + 1
  }
  const heading = ordered.find((row) => row.id === section)
  if (!heading) {
    return { refusal: 'no row with this id' }
  }
  if (heading.kind !== 'heading') {
    return { refusal: 'a section must be a heading row (MAIN-015)' }
  }
  const level = heading.headingLevel
  const next = ordered.find(
    (row) => row.kind === 'heading' && row.position > heading.position && row.headingLevel <= level,
  )
  return next ? next.position : ordered.length + 1
}

/** §6.9's cross-row clause: whether a VISIBLE instruction already
 * carries the name NOCASE-equal — the real store's
 * `active_title_taken` mirror, including its fold: SQLite's COLLATE
 * NOCASE folds ASCII A–Z only, so "Ä"/"ä" are NOT twins there and are
 * not here either (the mock stays exactly as permissive as the real
 * boundary, never inventing a rejection). `except` is the row the
 * check excludes (the store's own by-id exclusion; a HIDDEN row never
 * blocks). */
function visibleNameTaken(
  rows: readonly MockLibraryRow[],
  name: string,
  except?: MockLibraryRow,
): boolean {
  const wanted = asciiFold(name)
  return rows.some(
    (row) =>
      row !== except &&
      row.kind === 'instruction' &&
      row.state === 'active' &&
      asciiFold(row.title) === wanted,
  )
}

/** SQLite's NOCASE fold: ASCII letters only. */
const asciiFold = (value: string): string => value.replace(/[A-Z]/g, (c) => c.toLowerCase())

/** The shared by-id refusal of the three id-addressed commands (the
 * real `instruction_of`'s twin): an unknown id or a heading row is the
 * typed `E_VALIDATION` refusal of the `id` field. */
function instructionRowOf(state: MockState, id: string): MockLibraryRow | Promise<never> {
  const row = state.library.find((item) => item.id === id)
  if (!row || row.kind !== 'instruction') {
    return rejectWith('E_VALIDATION', 'errors.validation', {
      field: 'id',
      value: boundedEcho(id),
      reason: row
        ? 'the row is a heading; headings have no instruction lifecycle (MAIN-015)'
        : 'no instruction with this id',
    })
  }
  return row
}

// ---------------------------------------------------------------------------
// The four structure commands (T-330): mock twins over the union-shaped
// library rows. The backend owns ALG-060/061/062; these helpers mirror the
// observable contracts for UI tests (semantic targets, all-or-nothing level
// validation, dense positions and one journal entry per real change).
// ---------------------------------------------------------------------------

type HeadingRow = Extract<MockLibraryRow, { kind: 'heading' }>

function isHeading(row: MockLibraryRow): row is HeadingRow {
  return row.kind === 'heading'
}

/** The closed heading-level vocabulary after arithmetic has been checked. */
function headingLevel(value: number): HeadingRow['headingLevel'] | null {
  if (value === 2 || value === 3 || value === 4 || value === 5 || value === 6) return value
  return null
}

/** The position-ordered rows the structure commands operate on. */
function orderedLibrary(state: MockState): MockLibraryRow[] {
  return [...state.library].sort((left, right) => left.position - right.position)
}

/** A row lookup with the real command's field-scoped, content-free refusal. */
function structureRowIndex(
  rows: readonly MockLibraryRow[],
  id: string,
  field: 'id' | 'target' | 'parent',
): number | Promise<never> {
  const index = rows.findIndex((row) => row.id === id)
  if (index < 0) {
    return rejectWith('E_VALIDATION', 'errors.validation', {
      field,
      reason: 'no library row has this id',
    })
  }
  return index
}

/** A heading-only lookup; an instruction id is the same typed refusal. */
function structureHeadingIndex(
  rows: readonly MockLibraryRow[],
  id: string,
  field: 'id' | 'target' | 'parent',
): number | Promise<never> {
  const index = structureRowIndex(rows, id, field)
  if (index instanceof Promise) return index
  if (!isHeading(rows[index]!)) {
    return rejectWith('E_VALIDATION', 'errors.validation', {
      field,
      reason: 'the row is not a heading',
    })
  }
  return index
}

/** ALG-060's original-list branch range, hidden instructions included. */
function mockBranchRange(
  rows: readonly MockLibraryRow[],
  headingIndex: number,
): { start: number; end: number } | Promise<never> {
  const root = rows[headingIndex]
  if (!root || !isHeading(root)) {
    return rejectWith('E_VALIDATION', 'errors.validation', {
      field: 'id',
      reason: 'the requested heading branch could not be resolved',
    })
  }
  let end = rows.length
  for (let index = headingIndex + 1; index < rows.length; index++) {
    const row = rows[index]
    if (row && isHeading(row) && row.headingLevel <= root.headingLevel) {
      end = index
      break
    }
  }
  return { start: headingIndex, end }
}

/** ALG-061's original-list cut-and-insert. The caller checks the interior. */
function mockMoveRange(
  rows: readonly MockLibraryRow[],
  start: number,
  end: number,
  target: number,
): MockLibraryRow[] {
  if (target >= end) {
    return [
      ...rows.slice(0, start),
      ...rows.slice(end, target),
      ...rows.slice(start, end),
      ...rows.slice(target),
    ]
  }
  return [
    ...rows.slice(0, target),
    ...rows.slice(start, end),
    ...rows.slice(target, start),
    ...rows.slice(end),
  ]
}

/** Renumbers the complete table-wide sequence (DB-005/DB-011). */
function renumberLibrary(rows: MockLibraryRow[]): void {
  rows.forEach((row, index) => {
    row.position = index + 1
  })
}

/** Maps one heading row to the generated command DTO. */
function toHeadingRecord(row: HeadingRow): HeadingRecordWire {
  return {
    id: row.id,
    title: row.title,
    level: row.headingLevel,
    position: row.position,
    created_at: row.createdAt,
    updated_at: row.updatedAt,
  }
}

/** §6.9 heading-name validation, with Rust trim and one positional scan. */
function headingNameRejection(name: string): Promise<never> | null {
  const trimmed = rustTrim(name)
  const characters = Array.from(trimmed)
  if (characters.length === 0) {
    return rejectWith('E_VALIDATION', 'errors.validation', {
      field: 'name',
      reason: 'a heading name needs 1-200 characters after trimming',
    })
  }
  if (characters.length > 200) {
    return rejectWith('E_VALIDATION', 'errors.validation', {
      field: 'name',
      reason: 'a heading name is at most 200 characters',
      length: String(characters.length),
    })
  }
  const offense = firstNameOffense(trimmed)
  if (offense) {
    return rejectWith('E_VALIDATION', 'errors.validation', {
      field: 'name',
      reason:
        offense.kind === 'nul'
          ? 'a heading name must not contain NUL'
          : 'a heading name is one line',
      index: String(offense.index),
      character: offense.character,
    })
  }
  return null
}

/** Whether two ordered worlds have the same ids and heading levels. */
function sameStructure(left: readonly MockLibraryRow[], right: readonly MockLibraryRow[]): boolean {
  return left.every((row, index) => {
    const other = right[index]
    return (
      other !== undefined &&
      row.id === other.id &&
      (!isHeading(row) || (isHeading(other) && row.headingLevel === other.headingLevel))
    )
  })
}

// ---------------------------------------------------------------------------
// The command surface
// ---------------------------------------------------------------------------

/** Whether a variant's stored text outputs no text after ALG-002's
 * normalization (D-13's empty variant) — the mock twin of the Rust
 * `variant_text(content) === ""`. The normalizer's edge rules are
 * POSITIONAL: from the left it strips BOM runs and blank lines (each
 * a run of ASCII whitespace ended by a line break, the two
 * interleaved), from the right a run of ASCII whitespace — so the
 * twin is that same consumption as a regex, not a flat
 * "every character is BOM-or-whitespace" test (round-2 P3-1: a
 * TRAILING BOM blocks the right-hand strip — `" \u{FEFF}"` outputs
 * text, a leading one behind blank lines does not). The character
 * set is Rust's `char::is_ascii_whitespace` (five members) plus the
 * BOM; `\r` is consumed as whitespace because the normalizer's
 * line-break conversion turns it into `\n` first. */
const OUTPUTS_NO_TEXT = /^(\u{FEFF}|[ \t\f\r ]*\n)*[ \t\f\r ]*$/u
const outputsNoText = (content: string): boolean => OUTPUTS_NO_TEXT.test(content)

export const mockCommands: IpcCommands = {
  /** `projects_list` (§7.10): column order is `sort_order` (ALG-001). */
  async projectsList(includeArchived) {
    const rows = [...mockState().projects].sort((a, b) => a.sortOrder - b.sortOrder)
    const visible = includeArchived === true ? rows : rows.filter((p) => p.state === 'active')
    return visible.map(toColumnEntry)
  },

  /**
   * `library_get` (§7.10, T-083/D-187): the tree in `position` order —
   * instructions (name, variant count, dates, state, the variants
   * inline with LIB-014's emptiness flag and the main flag) and
   * headings (level, the LIB-032 skip flag) — with each instruction's
   * labels exactly the ACTIVE projects holding an assignment on it,
   * in the projects' `sort_order`, each naming the assigned variant.
   * The skip walk is the mock twin of the backend's gaps module (the
   * adjacent-heading rule, instructions transparent); the variant
   * half's known limit — the fixture set carries no soft-deleted
   * variants, so the count is always the whole list — lives in
   * `state.ts`'s `MockLibraryRow` note.
   */
  async libraryGet(includeHidden) {
    const state = mockState()
    // Sorted by the row's `position` — the wire contract's order —
    // not the array order the fixture builder happens to leave: a
    // test that reorders `mockState().library` (as the LIB-032 spec
    // below does) keeps the answer in position order.
    const rows = [...state.library]
      .sort((a, b) => a.position - b.position)
      .filter((row) => includeHidden === true || row.state === 'active')
    // LIB-032 over the SERVED rows: the heading that arrives more than
    // one level deeper than the heading before it is flagged — one
    // skip per offending edge, instructions of either state
    // transparent (the gaps module's own rule).
    const skipped = new Set<number>()
    // D-199: the second fact of the same pair — the skipped-from
    // level rides exactly the rows the flag marks; one walk builds
    // both, mirroring the backend's one-report rule.
    const fromLevels = new Map<number, number>()
    let previous: { level: number } | null = null
    rows.forEach((row, index) => {
      if (row.kind !== 'heading') return
      if (previous !== null && row.headingLevel > previous.level + 1) {
        skipped.add(index)
        fromLevels.set(index, previous.level)
      }
      previous = { level: row.headingLevel }
    })
    // The label join (D-187): the active projects in `sort_order` are
    // walked in that order, so every instruction's labels come out in
    // column order; an assignment whose project is not in the active
    // set labels nothing (DB-014: archived rows are kept, never
    // shown).
    const activeProjects = [...state.projects]
      .sort((a, b) => a.sortOrder - b.sortOrder)
      .filter((row) => row.state === 'active')
    const assignmentsByProject = new Map<string, typeof state.assignments>()
    for (const assignment of state.assignments) {
      const list = assignmentsByProject.get(assignment.projectId) ?? []
      list.push(assignment)
      assignmentsByProject.set(assignment.projectId, list)
    }
    const labelsByInstruction = new Map<string, ProjectLabelEntry[]>()
    for (const project of activeProjects) {
      for (const assignment of assignmentsByProject.get(project.id) ?? []) {
        const labels = labelsByInstruction.get(assignment.instructionId) ?? []
        labels.push({
          project_id: project.id,
          name: project.name,
          label: project.label,
          color: project.color,
          variant_id: assignment.target.kind === 'variant' ? assignment.target.value : null,
        })
        labelsByInstruction.set(assignment.instructionId, labels)
      }
    }
    const tree = rows.map((row, index) => {
      if (row.kind === 'heading') {
        return {
          kind: 'heading' as const,
          id: row.id,
          title: row.title,
          level: row.headingLevel,
          level_skipped: skipped.has(index),
          from_level: fromLevels.get(index) ?? null,
        }
      }
      return {
        kind: 'instruction' as const,
        id: row.id,
        title: row.title,
        state: row.state,
        // DB-012's read side: only the ACTIVE variants count and serve —
        // the variant commands (T-329) put soft-deleted rows into the
        // same list, and the real `library_get` never shows one (the
        // landing's one mechanical touch to this twin).
        variant_count: row.variants.filter((variant) => variant.state === 'active').length,
        created_at: row.createdAt,
        updated_at: row.updatedAt,
        variants: row.variants
          .filter((variant) => variant.state === 'active')
          .map((variant) => ({
            id: variant.id,
            name: variant.name,
            is_main: variant.isMain,
            is_empty: outputsNoText(variant.content),
          })),
        project_labels: labelsByInstruction.get(row.id) ?? [],
      }
    })
    // D-200: the D-14 number rides the default answer — the count of
    // hidden instructions, scope-independent, from this mock's own
    // rows (no core here; the count is a filter, not a second read).
    const hiddenCount = state.library.filter(
      (row) => row.kind === 'instruction' && row.state === 'hidden',
    ).length
    return { tree, hidden_count: hiddenCount }
  },
  /** `heading_create` (§7.10, T-092): validated name/level, semantic placement, one journal row. */
  async headingCreate(name, level, parent) {
    const rejection = headingNameRejection(name)
    if (rejection) return rejection
    const validLevel = headingLevel(level)
    if (!validLevel) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'level',
        reason: 'heading levels must stay within 2 to 6',
        level: String(level),
      })
    }
    const state = mockState()
    const rows = orderedLibrary(state)
    let target = rows.length + 1
    if (parent !== null) {
      const index = structureHeadingIndex(rows, parent, 'parent')
      if (index instanceof Promise) return index
      const branch = mockBranchRange(rows, index)
      if (branch instanceof Promise) return branch
      target = branch.end + 1
    }
    const now = new Date().toISOString()
    const row: HeadingRow = {
      kind: 'heading',
      id: freshRowId(),
      position: 0,
      headingLevel: validLevel,
      title: rustTrim(name),
      state: 'active',
      createdAt: now,
      updatedAt: now,
    }
    rows.splice(target - 1, 0, row)
    renumberLibrary(rows)
    state.library = rows
    journal('heading.created', { entityType: 'heading', entityId: row.id })
    return toHeadingRecord(row)
  },

  /** `heading_update` (§7.10, T-092/T-094): one typed operation, all-or-nothing level rules. */
  async headingUpdate(id, update: HeadingUpdate) {
    if (update.mode === 'rename') {
      const rejection = headingNameRejection(update.value)
      if (rejection) return rejection
    }
    const state = mockState()
    const index = structureHeadingIndex(state.library, id, 'id')
    if (index instanceof Promise) return index
    const row = state.library[index]
    if (!row || !isHeading(row)) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'id',
        reason: 'the row is not a heading',
      })
    }
    const now = new Date().toISOString()
    if (update.mode === 'rename') {
      const renamed = rustTrim(update.value)
      if (renamed === row.title) return toHeadingRecord(row)
      row.title = renamed
      row.updatedAt = now
      journal('heading.changed', { entityType: 'heading', entityId: row.id })
      return toHeadingRecord(row)
    }
    if (update.mode === 'change_level') {
      const validLevel = headingLevel(update.value)
      if (!validLevel) {
        return rejectWith('E_VALIDATION', 'errors.validation', {
          field: 'level',
          reason: 'heading levels must stay within 2 to 6',
          level: String(update.value),
          index: String(orderedLibrary(state).findIndex((candidate) => candidate.id === id)),
        })
      }
      if (validLevel === row.headingLevel) return toHeadingRecord(row)
      row.headingLevel = validLevel
      row.updatedAt = now
      journal('heading.changed', { entityType: 'heading', entityId: row.id })
      return toHeadingRecord(row)
    }
    if (update.value !== -1 && update.value !== 1) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'branch_shift',
        reason: 'a manual branch level shift must be exactly -1 or +1',
        delta: String(update.value),
      })
    }
    const rows = orderedLibrary(state)
    const orderedIndex = rows.findIndex((candidate) => candidate.id === id)
    const branch = mockBranchRange(rows, orderedIndex)
    if (branch instanceof Promise) return branch
    const levels = new Map<number, HeadingRow['headingLevel']>()
    for (let position = branch.start; position < branch.end; position++) {
      const candidate = rows[position]
      if (!candidate || !isHeading(candidate)) continue
      const shifted = headingLevel(candidate.headingLevel + update.value)
      if (!shifted) {
        return rejectWith('E_VALIDATION', 'errors.validation', {
          field: 'branch_shift',
          reason: 'the whole branch was refused because a heading would leave 2 to 6',
          index: String(orderedIndex),
          delta: String(update.value),
          offender: String(position),
          offender_level: String(candidate.headingLevel),
        })
      }
      levels.set(position, shifted)
    }
    state.library = rows.map((candidate, position) => {
      const shifted = levels.get(position)
      return shifted !== undefined && isHeading(candidate)
        ? { ...candidate, headingLevel: shifted, updatedAt: now }
        : candidate
    })
    journal('heading.changed', {
      entityType: 'heading',
      entityId: id,
      details: JSON.stringify({ changed: levels.size }),
    })
    const stored = state.library.find((candidate) => candidate.id === id)
    if (!stored || !isHeading(stored)) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'id',
        reason: 'the updated heading disappeared from the library',
      })
    }
    return toHeadingRecord(stored)
  },

  /** `heading_delete` (§7.10, T-093): physical delete, dense positions, instructions kept. */
  async headingDelete(id) {
    const state = mockState()
    const index = structureHeadingIndex(state.library, id, 'id')
    if (index instanceof Promise) return index
    state.library.splice(index, 1)
    renumberLibrary(state.library)
    journal('heading.deleted', { entityType: 'heading', entityId: id })
    return null
  },

  /** `items_move` (§7.10, T-095): semantic drop targets, core-equivalent order/level guards. */
  async itemsMove(id, target: MoveTarget) {
    const state = mockState()
    const rows = orderedLibrary(state)
    const sourceIndex = structureRowIndex(rows, id, 'id')
    if (sourceIndex instanceof Promise) return sourceIndex
    const source = rows[sourceIndex]
    if (!source) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'id',
        reason: 'no library row has this id',
      })
    }
    const sourceIsHeading = isHeading(source)
    // Resolve the dragged range before alignment or insertion can change
    // which adjacent headings close it.
    let sourceEnd = sourceIndex + 1
    if (sourceIsHeading) {
      const branch = mockBranchRange(rows, sourceIndex)
      if (branch instanceof Promise) return branch
      sourceEnd = branch.end
    }
    let targetIndex = 0
    let alignedRootLevel: number | null = null
    if (target.mode === 'after_item') {
      const index = structureRowIndex(rows, target.id, 'target')
      if (index instanceof Promise) return index
      targetIndex = index + 1
    } else if (target.mode === 'into_heading') {
      const index = structureHeadingIndex(rows, target.id, 'target')
      if (index instanceof Promise) return index
      const targetRow = rows[index]
      if (!targetRow || !isHeading(targetRow)) {
        return rejectWith('E_VALIDATION', 'errors.validation', {
          field: 'target',
          reason: 'a drop target must be a heading row',
        })
      }
      if (sourceIsHeading && index >= sourceIndex && index < sourceEnd) {
        return rejectWith('E_VALIDATION', 'errors.validation', {
          field: 'target',
          reason: 'a branch cannot be dropped inside itself or its descendants',
        })
      }
      const targetBranch = mockBranchRange(rows, index)
      if (targetBranch instanceof Promise) return targetBranch
      targetIndex = targetBranch.end
      alignedRootLevel = sourceIsHeading ? targetRow.headingLevel + 1 : null
    } else {
      alignedRootLevel = sourceIsHeading ? 2 : null
    }

    if (sourceIndex < targetIndex && targetIndex < sourceEnd) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'target',
        reason: 'a branch cannot move inside its own range',
        range_start: String(sourceIndex),
        range_end: String(sourceEnd),
        target: String(targetIndex),
      })
    }
    let moveInput = rows
    if (alignedRootLevel !== null) {
      const root = rows[sourceIndex]
      if (!root || !isHeading(root)) {
        return rejectWith('E_VALIDATION', 'errors.validation', {
          field: 'id',
          reason: 'the requested heading branch could not be resolved',
        })
      }
      const delta = alignedRootLevel - root.headingLevel
      const levelUpdates = new Map<number, HeadingRow['headingLevel']>()
      for (let position = sourceIndex; position < sourceEnd; position++) {
        const candidate = rows[position]
        if (!candidate || !isHeading(candidate)) continue
        const aligned = headingLevel(candidate.headingLevel + delta)
        if (!aligned) {
          return rejectWith('E_VALIDATION', 'errors.validation', {
            field: 'heading_level',
            reason: 'the whole branch was refused because a heading would leave 2 to 6',
            index: String(sourceIndex),
            delta: String(delta),
            offender: String(position),
            offender_level: String(candidate.headingLevel),
          })
        }
        levelUpdates.set(position, aligned)
      }
      const now = new Date().toISOString()
      moveInput = rows.map((row, index) => {
        const aligned = levelUpdates.get(index)
        return aligned !== undefined && isHeading(row) && aligned !== row.headingLevel
          ? { ...row, headingLevel: aligned, updatedAt: now }
          : row
      })
    }
    let moved = mockMoveRange(moveInput, sourceIndex, sourceEnd, targetIndex)
    if (sameStructure(rows, moved)) return null
    moved = moved.map((row, index) => ({ ...row, position: index + 1 }))
    state.library = moved
    journal(sourceIsHeading ? 'heading.changed' : 'instruction.changed', {
      entityType: sourceIsHeading ? 'heading' : 'instruction',
      entityId: id,
      details: JSON.stringify({ changed: sourceEnd - sourceIndex }),
    })
    return null
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
    const row = appendProjectRow(state, key, 'manual')
    return { outcome: 'added', project: toAddedProject(row), offered_root: null, existing: null }
  },

  /**
   * `project_archive` (§7.10 Projects module, T-108/D-194): id-addressed —
   * a stale id is the typed `E_VALIDATION` refusal naming the field, with
   * nothing changed; a hit flips the row to `archived` (readable back
   * through `projectsList` behind the include flag), a scan-born row's
   * path joins the exclusions (readable back through `exclusionsList`,
   * the write-then-read fidelity rule) — converging over an existing row
   * of the same path key — and the one `project.archived` journal entry
   * lands where `activityList` reads it. A re-send on an archived row is
   * the real command's converging no-op: nothing changes, nothing is
   * journaled. The `E_SYNC_RUNNING` gate is not modelled (the `syncRun`
   * stand-in's own documented limit: no flag state exists here).
   */
  async projectArchive(projectId) {
    const state = mockState()
    const row = state.projects.find((row) => row.id === projectId)
    if (!row) {
      // D-115's cap, the mock twin of the Rust `bounded_echo` the real
      // refusal uses (commands.rs documents "the mock layer mirrors
      // the same cap"): an id from the webview is unbounded by
      // definition, and it must not ride the envelope whole.
      return rejectWith('E_VALIDATION', 'validation.invalid_value', {
        field: 'project_id',
        value: boundedEcho(projectId),
        reason: 'no project has this id',
      })
    }
    if (row.state === 'archived') {
      return null
    }
    row.state = 'archived'
    if (row.addedVia === 'scan') {
      const key = pathKeyOf(row.path)
      const existing = state.exclusions.find((exclusion) => exclusion.path_key === key)
      if (!existing) {
        state.exclusions.push({
          id: freshRowId(),
          path: row.path,
          path_key: key,
          reason: 'removed_scanned',
          created_at: new Date().toISOString(),
        })
      }
    }
    journal('project.archived', {
      entityType: 'project',
      entityId: row.id,
      projectId: row.id,
    })
    return null
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

  /**
   * `instruction_create` (§7.10 Library module, LIB-002, §8 S3,
   * T-328/D-188): the create dialog's save — §6.9 name-then-text
   * validation (the marker line answers `E_CONTENT_RESERVED`, its own
   * §10 row), the NOCASE-twin refusal among VISIBLE instructions, the
   * section placement (the end of the picked heading's range; `null`
   * is the root prefix end), the `Main` pair, and the one
   * `instruction.created` journal entry and the instruction/Main create
   * versions (T-331's history read surface).
   */
  async instructionCreate(name, text, section) {
    const state = mockState()
    // §6.9 first (D-076's split, mirrored): the name, then the text —
    // the same fixed order the real command runs.
    const validated = validateInstructionName(name)
    if ('reason' in validated) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'name',
        value: boundedEcho(name),
        reason: validated.reason,
      })
    }
    const textRejection = variantTextRejection(text)
    if (textRejection) {
      if (textRejection.kind === 'marker') {
        return rejectWith('E_CONTENT_RESERVED', 'errors.content_reserved', {
          field: 'text',
          marker: textRejection.marker,
          line: String(textRejection.line),
        })
      }
      // No value echo: the instruction text is the one thing SEC-008
      // keeps out of envelopes (the real command's rule, mirrored).
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'text',
        reason: textRejection.reason,
      })
    }
    const ordered = [...state.library].sort((a, b) => a.position - b.position)
    // The section check precedes the duplicate check — the real
    // command's own order (its `section_target` runs before
    // `create_item` meets the store's twin): a request with both
    // faults answers about the section.
    const target = sectionTarget(ordered, section)
    if (typeof target !== 'number') {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'section',
        value: boundedEcho(section ?? ''),
        reason: target.refusal,
      })
    }
    if (visibleNameTaken(state.library, validated.trimmed)) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'name',
        value: boundedEcho(validated.trimmed),
        reason: 'a visible instruction already carries this name, case-insensitive (6.9)',
      })
    }
    // The pair (LIB-002): the instruction with its `Main` variant —
    // the text stored in the SAVE-FOLDED form (§6.9: CRLF becomes LF
    // on save; a lone CR stays, the store's own `fold_crlf` rule).
    // The dates follow the row-creating shape the seeding uses
    // (DB-002 UTC; runtime rows stamp the wall clock — the seeded
    // rows' one-second-per-position form is the generator's own).
    const now = new Date().toISOString()
    const row: MockLibraryRow = {
      kind: 'instruction',
      id: freshRowId(),
      position: 0,
      title: validated.trimmed,
      state: 'active',
      variants: [
        {
          id: freshRowId(),
          name: 'Main',
          content: text.replace(/\r\n/g, '\n'),
          isMain: true,
          // The variant-command columns (T-329's union extension): a
          // created row is active with its own dates.
          state: 'active',
          createdAt: now,
          updatedAt: now,
          deletedAt: null,
        },
      ],
      createdAt: now,
      updatedAt: now,
    }
    // The placement: the row takes the target position with the dense
    // renumber (DB-011) — the store's own append-then-move, mirrored.
    ordered.splice(target - 1, 0, row)
    ordered.forEach((item, index) => {
      item.position = index + 1
    })
    state.library = ordered
    journal('instruction.created', { entityType: 'instruction', entityId: row.id })
    await recordMockRevision(
      'instruction',
      row.id,
      'create',
      JSON.stringify({ name: row.title, state: row.state }),
    )
    const main = row.kind === 'instruction' ? row.variants[0] : undefined
    if (main) {
      await recordMockRevision(
        'variant',
        main.id,
        'create',
        JSON.stringify({ name: main.name, text: main.content }),
      )
    }
    return {
      instruction_id: row.id,
      variant_id: row.variants[0]?.id as string,
      position: row.position,
    }
  },

  /**
   * `instruction_update` (§7.10 Library module, T-328/D-188): the
   * rename — the same-name save is the verified no-op (nothing
   * written, nothing journaled), a NOCASE twin among visible
   * instructions and a heading or stale id are the typed refusals, and
   * a real rename journals `instruction.changed`.
   */
  async instructionUpdate(id, name) {
    const state = mockState()
    const validated = validateInstructionName(name)
    if ('reason' in validated) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'name',
        value: boundedEcho(name),
        reason: validated.reason,
      })
    }
    const row = instructionRowOf(state, id)
    if (row instanceof Promise) return row
    // D-188's verified no-op: the same name writes and journals
    // nothing (the D-115 precedent).
    if (row.title === validated.trimmed) return null
    if (visibleNameTaken(state.library, validated.trimmed, row)) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'name',
        value: boundedEcho(validated.trimmed),
        reason: 'a visible instruction already carries this name, case-insensitive (6.9)',
      })
    }
    row.title = validated.trimmed
    journal('instruction.changed', { entityType: 'instruction', entityId: row.id })
    await recordMockRevision(
      'instruction',
      row.id,
      'update',
      JSON.stringify({ name: row.title, state: row.state }),
    )
    return null
  },

  /**
   * `instruction_hide` (§7.10 Library module, LIB-005, D-11/DB-013,
   * T-328/D-188): the hide — the row flips to `hidden` keeping its
   * variants (and whatever assignment state a future task wires in;
   * the real command touches none, DB-013), a repeated hide is the
   * verified no-op, and the one real transition journals
   * `instruction.hidden`.
   */
  async instructionHide(id) {
    const state = mockState()
    const row = instructionRowOf(state, id)
    if (row instanceof Promise) return row
    if (row.state === 'hidden') return null
    row.state = 'hidden'
    journal('instruction.hidden', { entityType: 'instruction', entityId: row.id })
    await recordMockRevision(
      'instruction',
      row.id,
      'hide',
      JSON.stringify({ name: row.title, state: row.state }),
    )
    return null
  },

  /**
   * `instruction_restore` (§7.10 Library module, LIB-005…007/012,
   * T-328/D-188): the restore — the row returns to `active`, a
   * repeated restore is the verified no-op, and a visible NOCASE twin
   * that took the name while the row was hidden refuses the restore
   * with the class-only envelope (the stored title never travels,
   * SEC-008). The one real transition journals
   * `instruction.restored`.
   */
  async instructionRestore(id) {
    const state = mockState()
    const row = instructionRowOf(state, id)
    if (row instanceof Promise) return row
    if (row.state === 'active') return null
    if (visibleNameTaken(state.library, row.title)) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'name',
        reason: 'a visible instruction already carries this name, case-insensitive (6.9)',
      })
    }
    row.state = 'active'
    journal('instruction.restored', { entityType: 'instruction', entityId: row.id })
    await recordMockRevision(
      'instruction',
      row.id,
      'restore',
      JSON.stringify({ name: row.title, state: row.state }),
    )
    return null
  },

  /** `markdown_preview` (§7.10, SEC-005/D-15): sanitized HTML + warnings. */
  async markdownPreview(text) {
    return renderMarkdownPreview(text)
  },

  /**
   * `scan_start` (§7.10, §9.6, PRJ-002; T-076/D-153): a stand-in over
   * the mock state, not a walker mirror (D-108's limits posture — no
   * filesystem): the scan root is a trusted string, and the "walk"
   * classifies the mock store by path prefix — every project row
   * under the root keeps its registry class (`known` for active,
   * `archived` for archived rows), every exclusion row under it is
   * `excluded`, and one synthetic `new` repository
   * (`<root>/discovered-repo`) keeps the "New" group non-empty. The
   * walk itself is a short microtask step loop (the yield is what
   * makes `scanCancel` observable mid-scan); each step checks the
   * cancel latch and emits one `scan://progress` payload, and a
   * latched cancel answers the payload-less `cancelled` outcome
   * (ERR-002: nothing is added to the store either way — adding is
   * `scanCommit`'s job). A COMPLETED walk publishes its reported keys
   * as the session's discovered set, the only paths `scanCommit`
   * admits (SEC-003's rule, D-179); a cancelled one leaves the
   * previous set untouched. A second start while one is live rejects
   * `E_SYNC_RUNNING` (the slot guard's mock twin). NOT modelled, like
   * `project_add_manual`'s list: the SEC-003 gate on the ROOT (paths
   * are trusted) — only the empty path is rejected, with the real
   * command's `E_VALIDATION` field envelope.
   */
  async scanStart(path: string): Promise<ScanRunOutcomeWire> {
    // The live-scan guard precedes input validation, mirroring the
    // real command's order (the slot publication is its first act):
    // with a scan live, "wait" is the answer whatever the input is.
    const state = mockState()
    if (state.scanning) {
      return rejectWith('E_SYNC_RUNNING', 'errors.sync_running')
    }
    if (path.trim() === '') {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'path',
        value: path,
        reason: 'the scan root must not be empty',
      })
    }
    state.scanning = true
    state.scanCancelled = false
    try {
      return await runMockScan(path)
    } finally {
      state.scanning = false
      state.scanCancelled = false
    }
  },

  /**
   * `scan_cancel` (§7.10, NFR-015; T-076/D-153): flips the live
   * scan's latch — the mock walk observes it on its next step and
   * answers `cancelled`. With nothing live it is the documented
   * idempotent no-op (the window-close race must not error).
   */
  async scanCancel() {
    const state = mockState()
    if (state.scanning) {
      state.scanCancelled = true
    }
    return null
  },

  /**
   * `scan_commit` (§7.10, PRJ-002's "Add selected"; T-077/D-179): the
   * discovered-set rule first — every sent path's key must be among the
   * last COMPLETED `scanStart`'s reported keys, else the typed
   * `E_VALIDATION` rejection of the `path` field (nothing is added); an
   * empty entry is the same field's rejection. Then one entry per
   * DISTINCT path in sent order: a path an ACTIVE project holds answers
   * `already_added` naming that row, one an ARCHIVED project holds
   * `archived` with the same row (the restore offer's id); every other
   * path — an excluded one INCLUDED — is appended through the shared add
   * step with `added_via = 'scan'` and answers `added` (D-196: the
   * commit lifts the exclusion, journals `exclusion.removed` beside the
   * row's `project.added`, and `"excluded"` can no longer occur). The
   * typed outcomes are not errors and journal nothing; a re-sent
   * selection converges. The empty selection answers the typed no-op.
   * `probe_failed` is always `false` here: no filesystem exists to
   * probe (the D-108 limits posture this file's header records).
   */
  async scanCommit(paths) {
    const state = mockState()
    for (const path of paths) {
      if (path.trim() === '') {
        return rejectWith('E_VALIDATION', 'errors.validation', {
          field: 'path',
          value: path,
          reason: 'the repository path must not be empty',
        })
      }
      if (!state.scanFindings.includes(pathKeyOf(path))) {
        return rejectWith('E_VALIDATION', 'errors.validation', {
          field: 'path',
          value: path,
          reason: 'must be a repository the last completed scan of this session reported (SEC-003)',
        })
      }
    }
    const entries: ScanCommitEntryWire[] = []
    const seen = new Set<string>()
    for (const path of paths) {
      const key = pathKeyOf(path)
      if (seen.has(key)) continue
      seen.add(key)
      const existingRow = state.projects.find((row) => pathKeyOf(row.path) === key)
      if (existingRow) {
        const existing: ExistingProject = {
          id: existingRow.id,
          name: existingRow.name,
          path: existingRow.path,
          state: existingRow.state,
        }
        entries.push({
          path: key,
          outcome: existingRow.state === 'active' ? 'already_added' : 'archived',
          project: null,
          existing,
        })
        continue
      }
      const exclusionIndex = state.exclusions.findIndex((row) => row.path_key === key)
      if (exclusionIndex !== -1) {
        // D-196, mock side: the commit over an excluded path LIFTS the
        // exclusion — the row goes, the lift is journaled with the
        // created row as its `project_id` — and the path registers like
        // any other. The journal order mirrors the backend: the row's
        // `project.added` (inside `appendProjectRow`) first, then the
        // lift's `exclusion.removed`.
        const [removed] = state.exclusions.splice(exclusionIndex, 1)
        const row = appendProjectRow(state, key, 'scan')
        journal('exclusion.removed', {
          entityType: 'exclusion',
          entityId: removed?.id ?? null,
          projectId: row.id,
          details: JSON.stringify({ path: removed?.path }),
        })
        entries.push({
          path: key,
          outcome: 'added',
          project: toAddedProject(row),
          existing: null,
        })
        continue
      }
      entries.push({
        path: key,
        outcome: 'added',
        project: toAddedProject(appendProjectRow(state, key, 'scan')),
        existing: null,
      })
    }
    return { entries, probe_failed: false }
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
      // The lost-watch notice deliberately does NOT clear: the
      // backend's after-sync full pass re-arms every wanted
      // directory, and a standing refusal (the watch budget) simply
      // refuses again — the class is a fact about the WATCH, not
      // about the file's content (D-155).
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

  // -------------------------------------------------------------------------
  // The three assignment commands and two history reads (T-331).
  // -------------------------------------------------------------------------

  /** `assignment_set`: toggle one cell, defaulting to the active main. */
  async assignmentSet(projectId, instructionId, enabled, variantId) {
    if (!enabled && variantId !== null) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'variant_id',
        reason: 'a variant is only meaningful when enabling an assignment',
      })
    }
    const state = mockState()
    if (!state.projects.some((row) => row.id === projectId)) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'project_id',
        reason: 'no project has this id',
      })
    }
    const instruction = variantInstructionOf(state, instructionId)
    if (!instruction) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'instruction_id',
        reason: 'no instruction has this id',
      })
    }
    const current = assignmentOf(state, projectId, instructionId)
    if (!enabled) {
      if (!current) return null
      state.assignments = state.assignments.filter((row) => row !== current)
      journalAssignment(projectId, instructionId, 1)
      return null
    }
    if (current?.target.kind === 'custom' && variantId === null) {
      return toAssignmentRecord(current)
    }
    const targetVariant =
      variantId === null
        ? activeMainOf(instruction.variants)
        : instruction.variants.find((row) => row.id === variantId && row.state === 'active')
    if (!targetVariant) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: variantId === null ? 'instruction_id' : 'variant_id',
        reason:
          variantId === null
            ? 'the instruction has no active main variant (DB-010)'
            : 'the variant must be active and belong to the same instruction (DB-012)',
      })
    }
    const target: AssignmentTargetWire = { kind: 'variant', value: targetVariant.id }
    if (current?.target.kind === 'variant' && current.target.value === targetVariant.id) {
      return toAssignmentRecord(current)
    }
    const now = new Date().toISOString()
    if (current) {
      current.target = target
      current.updatedAt = now
      journalAssignment(projectId, instructionId, 1)
      return toAssignmentRecord(current)
    }
    const row: MockAssignmentRow = {
      projectId,
      instructionId,
      target,
      createdAt: now,
      updatedAt: now,
    }
    state.assignments.push(row)
    journalAssignment(projectId, instructionId, 1)
    return toAssignmentRecord(row)
  },

  /** `assignment_set_custom`: per-project text, never a cloned variant. */
  async assignmentSetCustom(projectId, instructionId, content) {
    if (content !== null) {
      const rejection = customTextRejection(content)
      if (rejection) return rejection
    }
    const state = mockState()
    if (!state.projects.some((row) => row.id === projectId)) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'project_id',
        reason: 'no project has this id',
      })
    }
    const instruction = variantInstructionOf(state, instructionId)
    if (!instruction) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'instruction_id',
        reason: 'no instruction has this id',
      })
    }
    const current = assignmentOf(state, projectId, instructionId)
    if (content === null) {
      if (!current) return null
      if (current.target.kind === 'variant') return toAssignmentRecord(current)
      const main = activeMainOf(instruction.variants)
      if (!main) {
        return rejectWith('E_VALIDATION', 'errors.validation', {
          field: 'instruction_id',
          reason: 'the instruction has no active main variant (DB-010)',
        })
      }
      current.target = { kind: 'variant', value: main.id }
      current.updatedAt = new Date().toISOString()
      journalAssignment(projectId, instructionId, 1)
      return toAssignmentRecord(current)
    }
    const folded = foldCrlf(content)
    if (current?.target.kind === 'custom' && current.target.value === folded) {
      return toAssignmentRecord(current)
    }
    const now = new Date().toISOString()
    if (current) {
      current.target = { kind: 'custom', value: folded }
      current.updatedAt = now
      journalAssignment(projectId, instructionId, 1)
      return toAssignmentRecord(current)
    }
    const row: MockAssignmentRow = {
      projectId,
      instructionId,
      target: { kind: 'custom', value: folded },
      createdAt: now,
      updatedAt: now,
    }
    state.assignments.push(row)
    journalAssignment(projectId, instructionId, 1)
    return toAssignmentRecord(row)
  },

  /** `assignments_bulk`: one row/column product, one journal entry. */
  async assignmentsBulk(instructionIds, projectIds, enabled, variantId) {
    const instructions = [...new Set(instructionIds)]
    const projects = [...new Set(projectIds)]
    if (variantId !== null && (!enabled || instructions.length !== 1)) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'variant_id',
        reason: 'a variant is only meaningful when enabling exactly one instruction row',
      })
    }
    const state = mockState()
    for (const projectId of projects) {
      if (!state.projects.some((row) => row.id === projectId)) {
        return rejectWith('E_VALIDATION', 'errors.validation', {
          field: 'project_id',
          reason: 'no project has this id',
        })
      }
    }
    const rows: (MockLibraryRow & { kind: 'instruction' })[] = []
    for (const instructionId of instructions) {
      const instruction = variantInstructionOf(state, instructionId)
      if (!instruction) {
        return rejectWith('E_VALIDATION', 'errors.validation', {
          field: 'instruction_id',
          reason: 'no instruction has this id',
        })
      }
      rows.push(instruction)
    }
    const mains = new Map<string, string>()
    if (enabled) {
      if (variantId !== null) {
        const instruction = rows[0]
        if (!instruction) {
          return rejectWith('E_VALIDATION', 'errors.validation', {
            field: 'instruction_id',
            reason: 'no instruction has this id',
          })
        }
        const variant = instruction.variants.find(
          (row) => row.id === variantId && row.state === 'active',
        )
        if (!variant) {
          return rejectWith('E_VALIDATION', 'errors.validation', {
            field: 'variant_id',
            reason: 'the variant must be active and belong to the same instruction (DB-012)',
          })
        }
        mains.set(instruction.id, variant.id)
      } else {
        for (const instruction of rows) {
          const main = activeMainOf(instruction.variants)
          if (!main) {
            return rejectWith('E_VALIDATION', 'errors.validation', {
              field: 'instruction_id',
              reason: 'the instruction has no active main variant (DB-010)',
            })
          }
          mains.set(instruction.id, main.id)
        }
      }
    }
    let changed = 0
    const already: { project_id: string; instruction_id: string }[] = []
    for (const instructionId of instructions) {
      for (const projectId of projects) {
        const current = assignmentOf(state, projectId, instructionId)
        if (enabled) {
          if (current) {
            already.push({ project_id: projectId, instruction_id: instructionId })
            continue
          }
          const now = new Date().toISOString()
          state.assignments.push({
            projectId,
            instructionId,
            target: { kind: 'variant', value: mains.get(instructionId) ?? '' },
            createdAt: now,
            updatedAt: now,
          })
          changed++
        } else if (current) {
          if (current.target.kind === 'custom') {
            already.push({ project_id: projectId, instruction_id: instructionId })
            continue
          }
          state.assignments = state.assignments.filter((row) => row !== current)
          changed++
        }
      }
    }
    if (changed > 0) journalAssignment(null, null, changed)
    const result: BulkAssignmentResultWire = { changed, already_assigned: already }
    return result
  },

  /** `revisions_list`: instruction plus all variants, newest first. */
  async revisionsList(instructionId) {
    const state = mockState()
    const instruction = variantInstructionOf(state, instructionId)
    if (!instruction) {
      const row = state.library.find((candidate) => candidate.id === instructionId)
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'instruction_id',
        reason:
          row?.kind === 'heading'
            ? 'a heading has no version history (LIB-026/6.2)'
            : 'no instruction has this id',
      })
    }
    const variantIds = new Set(instruction.variants.map((row) => row.id))
    return state.revisions
      .filter(
        (row) =>
          (row.entity_type === 'instruction' && row.entity_id === instructionId) ||
          (row.entity_type === 'variant' && variantIds.has(row.entity_id)),
      )
      .sort(
        (left, right) =>
          right.created_at.localeCompare(left.created_at) ||
          Number(right.rev_no) - Number(left.rev_no) ||
          right.id.localeCompare(left.id),
      )
      .map((row) => ({
        id: row.id,
        entity_type: row.entity_type,
        entity_id: row.entity_id,
        rev_no: row.rev_no,
        op: row.op,
        snapshot_hash: row.snapshot_hash,
        reverted_from: row.reverted_from,
        created_at: row.created_at,
      }))
  },

  /** `revision_get`: one full history row by revision id. */
  async revisionGet(id) {
    const row = mockState().revisions.find((revision) => revision.id === id)
    if (!row) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'id',
        reason: 'no revision has this id',
      })
    }
    return { ...row }
  },

  // -------------------------------------------------------------------------
  // The five variant commands (§7.10 Library module, T-329): mock twins of
  // the Rust commands over the wired library rows (D-189 reading 8) — the
  // same contracts (NOCASE uniqueness, the exactly-one-main invariant, the
  // delete/restore flow, the E_* envelopes) over the in-memory store.
  // Real changes also append history rows for T-331's `revisions_list` /
  // `revision_get`; the no-op detection compares the stored outcome, the
  // ALG-070 hash gate's mock twin.
  // -------------------------------------------------------------------------

  /**
   * `variant_create` (§7.10, T-087): the two content sources (a copy of
   * the instruction's current main, or empty) with the caller's §6.9
   * name; the NOCASE duplicate refusal.
   */
  async variantCreate(instructionId, name, source) {
    const nameRejectionResult = nameRejection(name)
    if (nameRejectionResult) return nameRejectionResult
    const trimmed = rustTrim(name)
    const state = mockState()
    const instruction = variantInstructionOf(state, instructionId)
    if (!instruction) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'instruction_id',
        value: instructionId,
        reason: 'no instruction with this id',
      })
    }
    // DB-010's constructive half, BOTH sources alike (the store rejects
    // every non-main insert when the instruction has no active main —
    // the Rust command's `MainRequired` arm):
    const main = activeMainOf(instruction.variants)
    if (!main) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'instruction_id',
        reason: 'the instruction has no active main variant (DB-010)',
      })
    }
    const content = source === 'copy_main' ? main.content : ''
    if (nameTaken(instruction.variants, trimmed, null)) {
      return rejectVariantName(
        'an active variant of this instruction already carries this name (case-insensitive)',
      )
    }
    const now = new Date().toISOString()
    const row: MockVariantRow = {
      id: freshRowId(),
      name: trimmed,
      content,
      isMain: false,
      state: 'active',
      createdAt: now,
      updatedAt: now,
      deletedAt: null,
    }
    instruction.variants.push(row)
    journal('variant.created', { entityType: 'variant', entityId: row.id })
    await recordMockRevision(
      'variant',
      row.id,
      'create',
      JSON.stringify({ name: row.name, text: row.content }),
    )
    return toVariantRecord(instruction, row)
  },

  /**
   * `variant_update` (§7.10, T-088): the §6.9 rules (CRLF fold, NUL, the
   * reserved marker lines, the 262,144-character limit), a save without
   * changes journaling nothing (ALG-070's mock twin over the stored
   * outcome), one `variant.changed` entry per real change.
   */
  async variantUpdate(id, name, content) {
    if (name !== null) {
      const nameRejectionResult = nameRejection(name)
      if (nameRejectionResult) return nameRejectionResult
    }
    if (content !== null) {
      const codePoints = Array.from(content).length
      if (codePoints > VARIANT_TEXT_LIMIT) {
        return rejectWith('E_VALIDATION', 'errors.validation', {
          field: 'variant_text',
          reason: `variant text is at most ${VARIANT_TEXT_LIMIT} characters`,
          length: String(codePoints),
        })
      }
      const nulAt = content.indexOf('\0')
      if (nulAt >= 0) {
        return rejectWith('E_VALIDATION', 'errors.validation', {
          field: 'variant_text',
          reason: 'variant text must not contain NUL',
          index: String(Array.from(content.slice(0, nulAt)).length),
        })
      }
      const reserved = findReservedLine(content)
      if (reserved) {
        return rejectWith('E_CONTENT_RESERVED', 'errors.content_reserved', {
          field: 'variant_text',
          line: String(reserved.line),
          marker: reserved.marker,
        })
      }
    }
    const state = mockState()
    const found = findVariantRow(state, id)
    if (!found) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'id',
        value: id,
        reason: 'no variant with this id',
      })
    }
    const { instruction, variant: row } = found
    if (row.state !== 'active') {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'id',
        value: id,
        reason: 'the variant is soft-deleted; restore it first',
      })
    }
    const newName = name === null ? row.name : rustTrim(name)
    const newContent = content === null ? row.content : foldCrlf(content)
    // The duplicate-name probe runs only when a name was sent (the
    // store checks `Some(name)` only — a fieldless or text-only save
    // never probes).
    if (name !== null && nameTaken(instruction.variants, newName, id)) {
      return rejectVariantName(
        'an active variant of this instruction already carries this name (case-insensitive)',
      )
    }
    if (newName === row.name && newContent === row.content) {
      // The verified no-op: nothing journaled (ALG-070, D-16) — but the
      // STORE still restamps `updated_at` for a field-present write
      // (`update_variant`'s own landed semantics, `library.rs`), so the
      // mock restamps too: a same-value save is a written row without a
      // real change, mock and real alike.
      if (name !== null || content !== null) {
        row.updatedAt = new Date().toISOString()
      }
      return toVariantRecord(instruction, row)
    }
    row.name = newName
    row.content = newContent
    row.updatedAt = new Date().toISOString()
    journal('variant.changed', { entityType: 'variant', entityId: row.id })
    await recordMockRevision(
      'variant',
      row.id,
      'update',
      JSON.stringify({ name: row.name, text: row.content }),
    )
    return toVariantRecord(instruction, row)
  },

  /**
   * `variant_set_main` (§7.10, T-089): demote-and-promote keeps exactly
   * one active main (DB-010); re-promoting the current main is the
   * journaled-nothing no-op.
   */
  async variantSetMain(id) {
    const state = mockState()
    const found = findVariantRow(state, id)
    if (!found) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'id',
        value: id,
        reason: 'no variant with this id',
      })
    }
    const { instruction, variant: row } = found
    if (row.state !== 'active') {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'id',
        value: id,
        reason: 'the variant is soft-deleted; restore it first',
      })
    }
    if (row.isMain) return toVariantRecord(instruction, row)
    for (const other of instruction.variants) {
      if (other.id !== row.id) other.isMain = false
    }
    row.isMain = true
    journal('variant.changed', { entityType: 'variant', entityId: row.id })
    return toVariantRecord(instruction, row)
  },

  /**
   * `variant_delete` (§7.10, T-090): the default replacement is the
   * instruction's main (LIB-023/D-125 C3), the named successor must be
   * an active sibling, the last active variant is refused whatever the
   * replacement names (LIB-025), and every assignment moves — none
   * disappears.
   */
  async variantDelete(id, replacement) {
    const state = mockState()
    const found = findVariantRow(state, id)
    if (!found) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'id',
        value: id,
        reason: 'no variant with this id',
      })
    }
    const { instruction, variant: row } = found
    if (row.state === 'deleted') return toVariantRecord(instruction, row)
    // LIB-025 first: the absolute refusal, whatever the replacement
    // names.
    const activeSiblings = instruction.variants.filter(
      (candidate) => candidate.id !== row.id && candidate.state === 'active',
    )
    if (activeSiblings.length === 0) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'id',
        value: id,
        reason: 'the last active variant of an instruction cannot be deleted',
      })
    }
    let successor: MockVariantRow
    if (replacement === null || replacement === undefined) {
      if (row.isMain) {
        return rejectWith('E_VALIDATION', 'errors.validation', {
          field: 'replacement',
          reason: 'deleting the main variant requires naming its successor',
        })
      }
      const main = activeMainOf(instruction.variants)
      if (!main) {
        return rejectWith('E_VALIDATION', 'errors.validation', {
          field: 'instruction_id',
          reason: 'the instruction has no active main variant (DB-010)',
        })
      }
      successor = main
    } else {
      const candidate = instruction.variants.find((row2) => row2.id === replacement)
      if (!candidate || candidate.id === row.id || candidate.state !== 'active') {
        return rejectWith('E_VALIDATION', 'errors.validation', {
          field: 'replacement',
          value: replacement,
          reason: 'the replacement is not an active variant of the same instruction',
        })
      }
      successor = candidate
    }
    const moved = state.assignments.filter(
      (assignment) => assignment.target.kind === 'variant' && assignment.target.value === row.id,
    ).length
    for (const assignment of state.assignments) {
      if (assignment.target.kind === 'variant' && assignment.target.value === row.id) {
        assignment.target = { kind: 'variant', value: successor.id }
        assignment.updatedAt = new Date().toISOString()
      }
    }
    const now = new Date().toISOString()
    const wasMain = row.isMain
    row.state = 'deleted'
    row.isMain = false
    row.deletedAt = now
    row.updatedAt = now
    if (wasMain) successor.isMain = true
    await recordMockRevision(
      'variant',
      row.id,
      'delete',
      JSON.stringify({ name: row.name, text: row.content }),
    )
    journal('variant.deleted', {
      entityType: 'variant',
      entityId: row.id,
      details: JSON.stringify({ changed: moved }),
    })
    return toVariantRecord(instruction, row)
  },

  /**
   * `variant_restore` (§7.10, T-091): the soft-deleted row returns to
   * `active` without its old main flag; a name reused by an active twin
   * is the `E_VALIDATION` refusal.
   */
  async variantRestore(id) {
    const state = mockState()
    const found = findVariantRow(state, id)
    if (!found) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'id',
        value: id,
        reason: 'no variant with this id',
      })
    }
    const { instruction, variant: row } = found
    if (row.state === 'active') return toVariantRecord(instruction, row)
    if (nameTaken(instruction.variants, row.name, id)) {
      return rejectVariantName(
        'an active variant of this instruction already carries this name (case-insensitive)',
      )
    }
    row.state = 'active'
    row.deletedAt = null
    row.updatedAt = new Date().toISOString()
    await recordMockRevision(
      'variant',
      row.id,
      'restore',
      JSON.stringify({ name: row.name, text: row.content }),
    )
    journal('variant.restored', { entityType: 'variant', entityId: row.id })
    return toVariantRecord(instruction, row)
  },

  // -------------------------------------------------------------------------
  // The Backups read (§7.10 Backups module, T-125, BKP-004): the mock
  // twin of the Rust `backups_list` — the same typed unknown-id
  // refusal, the same newest-first order (D-122's ordering with the id
  // tiebreak), over the seeded `backupRecords` rows. Read-only: no
  // journal entry, no row change. Known limit, deliberate (D-108's
  // posture): no mock command writes backup rows (the `syncRun`
  // stand-in does not model phase 2), so tests edit the rows
  // directly.
  // -------------------------------------------------------------------------

  /**
   * `backups_list` (§7.10, BKP-004): one project's whole backup-copy
   * history — date, file, size, sync run and status — every §6.2
   * CHECK status (§6.4 keeps the history as status), newest first. An
   * unknown project id is the typed `E_VALIDATION` refusal (the
   * `revisions_list` twin: a caller bug, never silently "no
   * backups"); a known project with no rows is the honest empty list.
   */
  async backupsList(projectId) {
    const state = mockState()
    if (!state.projects.some((row) => row.id === projectId)) {
      return rejectWith('E_VALIDATION', 'errors.validation', {
        field: 'project_id',
        reason: 'no project has this id',
      })
    }
    return state.backupRecords
      .filter((row) => row.projectId === projectId)
      .sort(
        (left, right) =>
          right.created_at.localeCompare(left.created_at) || right.id.localeCompare(left.id),
      )
      .map(toBackupCopyRecord)
  },
}

/**
 * The mock walk of [`mockCommands.scanStart`]: classifies the store by
 * path prefix, then delivers the terminal state over a short microtask
 * step loop — one `scan://progress` emission per step (the real
 * backend's ≥100 ms throttle has no mock twin: the six emissions ARE
 * the observable, and the rate bound is the Rust seam's test), the
 * cancel latch checked between every step (a latched cancel answers
 * the payload-less `cancelled` outcome with the store untouched), and
 * the completed walk's keys published as the session's discovered set
 * (the commit's SEC-003 universe).
 */
async function runMockScan(root: string): Promise<ScanRunOutcomeWire> {
  const state = mockState()
  const trimmed = root.trim().replace(/\/+$/, '')
  const prefix = trimmed + '/'
  // The root itself matches too: a registered project scanned AS the
  // root is under it (the real walk roots on the canonical path).
  const under = (path: string) => path === trimmed || path.startsWith(prefix)
  const repositories: ScannedRepository[] = state.projects
    .filter((row) => under(row.path))
    .map((row) => ({
      path: row.path,
      path_key: pathKeyOf(row.path),
      class: row.state === 'active' ? ('known' as const) : ('archived' as const),
    }))
  for (const exclusion of state.exclusions) {
    if (under(exclusion.path)) {
      repositories.push({
        path: exclusion.path,
        path_key: pathKeyOf(exclusion.path),
        class: 'excluded',
      })
    }
  }
  const discovered = `${prefix}discovered-repo`
  repositories.push({ path: discovered, path_key: pathKeyOf(discovered), class: 'new' })

  // The §6.5 defaults' first exclude name is the mock's one skipped
  // folder (the real walk skips every found repository's own `.git`
  // with this reason — one row carries the vocabulary here).
  const excludeNames = settingStrings(state.settings, 'scan.exclude_names')
  const skipped: ScanSkipWire[] =
    excludeNames.length > 0
      ? [{ path: `${prefix}${excludeNames[0]}`, reason: 'excluded', os_code: null }]
      : []
  const checked = repositories.length * 2 + skipped.length + 2

  for (let step = 1; step <= 6; step += 1) {
    // The inter-step yield is the seam `scanCancel` lands through; a
    // latched cancel ends the walk with the payload-less answer.
    await Promise.resolve()
    if (state.scanCancelled) {
      return { outcome: 'cancelled' }
    }
    const ratio = step / 6
    mockEvents.scanProgress.emit({
      checked: Math.round(checked * ratio),
      found: Math.round(repositories.length * ratio),
      current_path: step === 6 ? discovered : `${prefix}folder-${step}`,
    })
  }
  // The discovered set (D-179): only a completed walk publishes.
  state.scanFindings = repositories.map((repo) => repo.path_key)
  return {
    outcome: 'completed',
    checked,
    found: repositories.length,
    repositories,
    skipped,
  }
}

/** One §6.5 string-list setting's current values ([] when unset or
 * malformed — the value rides the wire as canonical JSON text, the
 * `SettingItem.value` contract). */
function settingStrings(settings: SettingItem[], key: string): string[] {
  const found = settings.find((item) => item.key === key)
  if (!found) return []
  try {
    const parsed: unknown = JSON.parse(found.value)
    return Array.isArray(parsed) && parsed.every((item) => typeof item === 'string')
      ? (parsed as string[])
      : []
  } catch {
    return []
  }
}

export { SETTING_VALUE_SHAPES }
