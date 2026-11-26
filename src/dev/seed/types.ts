/*
 * Seed types (T-363): the shape of the mock data the shell and the dev
 * gallery render. Ported from the frozen prototype's demo data — the
 * markup of docs/prototype/index.html (the matrix rows and assignment
 * cells) and the demo objects of docs/prototype/app.js (PROJECTS,
 * VARIANTS, SETTINGS). These are DATA shapes for the seed only; the
 * real wire types arrive through src/bindings.ts (tauri-specta) and
 * are never derived from this file.
 *
 * Family cards (T-364…T-367) extend this module with the shapes their
 * screens need (activity entries, help articles, scan results, sync
 * summary rows, archived projects, exclusions) — leave room, do not
 * reshape what is here without checking their cards.
 */

/** Where a project came from (demo `addedVia` values). */
export type ProjectOrigin = 'manual' | 'scan'

/** The managed-block scheme (spec 5.x; demo `scheme`). */
export type ProjectScheme = 'A' | 'B' | 'C'

/**
 * Which position an extra file takes in the built document (demo
 * `extraLocalPos`/`extraProjectPos`).
 */
export type ExtraFilePosition = 'start' | 'end'

/**
 * The per-project column state the mock-up header shows as a sub-line:
 * ok (the short last-sync date), dirty (unsynced changes), external
 * (modified outside the app), unavailable (folder unreachable), deleted
 * (target file gone). Derived from the mock-up's header markup; cells
 * of an unavailable project render disabled.
 */
export type ProjectStatus = 'ok' | 'dirty' | 'external' | 'unavailable' | 'deleted'

/** A tracked project (demo PROJECTS entry + the header's status line). */
export type Project = {
  readonly name: string
  /** The short uppercase column label (demo `label`). */
  readonly label: string
  /** The column color chip, 6-digit hex (demo `color` / PS_PALETTE). */
  readonly color: string
  readonly description: string
  /** Absolute path of the target repository (demo data). */
  readonly path: string
  readonly origin: ProjectOrigin
  readonly scheme: ProjectScheme
  readonly baseFile: string
  readonly extraLocal: boolean
  readonly extraLocalPos: ExtraFilePosition
  readonly extraProject: boolean
  readonly extraProjectPos: ExtraFilePosition
  /** The long last-sync stamp the header tooltip shows (demo `syncedAt`). */
  readonly syncedAt: string
  readonly runs: number
  readonly status: ProjectStatus
}

/** An alternative variant of an instruction (demo VARIANTS entries). */
export type Variant = {
  readonly name: string
  readonly text: string
}

/**
 * One library-tree heading row (`## General`, `### Communication` —
 * demo `row-heading`). `depth` is the tree indent the mock-up renders
 * via `--depth`: 0 is a top-level section, 1 a nested one.
 */
export type LibraryHeading = {
  readonly kind: 'heading'
  readonly name: string
  readonly depth: 0 | 1
}

/**
 * One instruction row (demo rows with `.name`). `depth` follows the
 * parent heading (1 under a top-level section, 2 under a nested one).
 * `text: null` is the empty instruction (MAIN-009: the text is not
 * written yet — the mock-up dims the name). `hidden` is LIB-005…007.
 * `variants` lists the ALTERNATIVE variants; the main variant is
 * implicit. The demo materializes `Variant 1` with the main text for
 * its "1 variant" rows (app.js builds it lazily) — the seed stores it
 * explicitly so every displayed count is data, not a rule.
 */
export type LibraryInstruction = {
  readonly kind: 'instruction'
  readonly name: string
  readonly depth: 1 | 2
  readonly text: string | null
  readonly hidden: boolean
  readonly variants: readonly Variant[]
}

/** A library row in document order: a heading or an instruction. */
export type LibraryRow = LibraryHeading | LibraryInstruction

/**
 * One matrix cell: the assignment of an instruction to a project.
 * A discriminated union on `state`: only the `variant` state carries a
 * variant name, so an unnameable "variant assignment" cannot be built.
 * `dirty` marks a change made in the app that the next sync writes
 * (the mock-up's "(changed)" cells). The `custom` arm is the
 * per-project custom text (§11.2, MAIN-007/MAIN-034, T-364): `mode`
 * is the replace/extend radio of MAIN-034, and `base` names the base
 * variant of an extend cell — the variant assigned when "+" was
 * pressed, fixed while the cell stays custom (a replace cell has no
 * base). An EMPTY VARIANT is not a state of its own: it is a `main` or
 * `variant` assignment whose variant's text is empty (LIB-014/D-13) —
 * the rendering derives it (an instruction with `text: null` has an
 * empty main variant).
 */
export type Assignment =
  | { readonly state: 'none'; readonly dirty: boolean }
  | { readonly state: 'main'; readonly dirty: boolean }
  | { readonly state: 'variant'; readonly variant: string; readonly dirty: boolean }
  | {
      readonly state: 'custom'
      readonly mode: 'replace' | 'extend'
      readonly base?: string
      readonly dirty: boolean
    }

/** Assignments keyed by instruction name, then by project name. */
export type AssignmentMatrix = Readonly<Record<string, Readonly<Record<string, Assignment>>>>

/**
 * The operation recorded on a history revision (the demo's op vocabulary,
 * app.js ~2405: create / update / hide / restore / delete). The chip
 * class `hist-op--<op>` keys off it.
 */
export type HistoryOp = 'create' | 'update' | 'hide' | 'restore' | 'delete'

/**
 * The snapshot a revision carries (D-16: an instruction version stores
 * the name and the active/hidden state; a variant version the name and
 * the text — the main-variant flag and the library order are not part
 * of snapshots). The `kind` discriminant picks the snapshot body the
 * history panel renders.
 */
export type HistorySnapshot =
  | { readonly kind: 'instruction'; readonly name: string; readonly state: 'active' | 'hidden' }
  | { readonly kind: 'variant'; readonly name: string; readonly text: string }

/**
 * One revision row of the history panel (LIB-026): number, time,
 * operation, entity and snapshot. `revNo` is unique per entity; `when`
 * is the display stamp (the demo renders its fixed UTC+3 clock format,
 * app.js histStamp — the seed stores the rendered strings, like the
 * projects' `syncedAt`). `restored` is the demo's post-restore
 * feedback state (a delete row flips to restore + the chip, LIB-027);
 * the seed data starts with none.
 */
export type HistoryRevision = {
  readonly entityType: 'instruction' | 'variant'
  /** The variant's name; unused for instruction rows (the cell shows
   * the word "Instruction"). */
  readonly entityName: string
  readonly op: HistoryOp
  readonly revNo: number
  readonly when: string
  readonly snapshot: HistorySnapshot
  readonly restored: boolean
}

/**
 * One typed block of the static Preview sample the editor's Preview tab
 * renders (T-365): the tab shows template markup shaped like
 * markdown_preview's output (headings, paragraphs, lists, a code block)
 * without any renderer here — the real rendering is markdown_preview's
 * (7.10, SEC-005), a behavior card's. The seed data rides the same
 * DEV-only branch as the rest of the bundle.
 */
export type PreviewBlock =
  | { readonly kind: 'heading'; readonly level: 2 | 3 | 4; readonly text: string }
  | { readonly kind: 'paragraph'; readonly text: string }
  | { readonly kind: 'code'; readonly text: string }
  | { readonly kind: 'list'; readonly items: readonly string[] }

/*
 * The projects family's shapes (T-366): the scan results, the archived
 * projects, the exclusions and the Backups-tab rows — all DATA shapes
 * for the seed only (the real wire types arrive through
 * src/bindings.ts).
 */

/** One "New" row of the scan results (a repository to add — PRJ-002). */
export type ScanNewRow = {
  readonly name: string
  /** The row's rendered display path (root-prefixed, like syncedAt). */
  readonly path: string
}

/** One "Already added" row (no checkbox — PRJ-002). */
export type ScanKnownRow = {
  readonly name: string
  readonly path: string
}

/**
 * The closed skip-reason vocabulary of the scan results (PRJ-002:
 * excluded, submodule or worktree, no access, non-UTF-8 path, archived
 * or in exclusions). The reason's badge wording and its fix hint are
 * catalog copy keyed by this value.
 */
export type ScanSkipReason =
  'excluded' | 'submodule' | 'noAccess' | 'nonUtf8' | 'archived' | 'inExclusions'

/** One "Skipped" row: the path and the reason (PRJ-002, PRJ-007). */
export type ScanSkipRow = {
  readonly path: string
  readonly reason: ScanSkipReason
}

/**
 * One archived-project row of the Archived-projects screen (demo
 * ARCHIVED, app.js ~4164). `origin` drives the same wording the
 * settings modal's "Added" row uses. `color` is a DB-040 palette hex.
 */
export type ArchivedProject = {
  readonly name: string
  readonly path: string
  readonly color: string
  /** The "Archived <date>" display stamp (the demo's rendered string). */
  readonly archivedAt: string
  readonly origin: ProjectOrigin
}

/** The §6.2 `project_exclusions` reason enum (DDL mirror). */
export type ExclusionReason = 'removed_scanned' | 'manual'

/** One excluded-path row (demo EXCLUSIONS, DDL shape: path, reason,
 * created stamp; app.js ~4271). The badge wording is catalog copy
 * keyed by `reason`. */
export type ExcludedPath = {
  readonly path: string
  readonly reason: ExclusionReason
  /** The "Added <date>" display stamp. */
  readonly addedAt: string
}

/**
 * One Backups-tab row (BKP-004: date, file, size, sync run number).
 * The demo builds the rows from the project's syncedAt (app.js
 * renderBackups, ~3049) — JS is never ported, so the seed stores that
 * OUTPUT: display strings, plus `date` (the day part the BKP-012
 * tooltip names beside the file).
 */
export type BackupRow = {
  readonly date: string
  readonly when: string
  readonly file: string
  readonly size: string
  /** The "#47" run label (display string). */
  readonly run: string
}

/*
 * The service screens' shapes (T-367): the sync summary and pending
 * rows, the activity entries and the help articles — all DATA shapes
 * for the seed only (the real wire types arrive through
 * src/bindings.ts; the About facts are plain seed strings in
 * ./about.ts and carry no shape here).
 */

/** The §6.2 sync_run_projects status enum (the summary's four groups). */
export type SyncStatus = 'written' | 'unchanged' | 'skipped' | 'failed'

/**
 * One per-project row of the sync run summary (OUT-009, UI-011). A
 * discriminated union on `status`: only the skipped and failed rows
 * carry the `E_*` code and the what-to-do hint (UI-011's what-to-do
 * answer). `targetFile` is the row's rendered display path (the demo
 * targetPathOf's output — "~"-prefixed, like the projects' syncedAt
 * stamps).
 */
export type SyncSummaryRow =
  | {
      readonly status: 'written' | 'unchanged'
      readonly project: string
      readonly targetFile: string
      readonly reason: string
    }
  | {
      readonly status: 'skipped' | 'failed'
      readonly project: string
      readonly targetFile: string
      readonly reason: string
      readonly code: string
      readonly hint: string
    }

/**
 * One row of the "Unsynced changes" view (MAIN-026): the dirty project
 * and its target file (scheme C writes `<STEM>.project.md`, §9.4).
 */
export type SyncPendingRow = {
  readonly project: string
  readonly targetFile: string
}

/** The §7.7 entry type groups (the Type filter's values). */
export type ActivityType = 'projects' | 'library' | 'assignments' | 'sync' | 'backups'

/** The §7.7 result enum (the ok / skipped / error chips). */
export type ActivityResult = 'ok' | 'skipped' | 'error'

/**
 * One activity entry (LOG-001…005): time, type, action, object and
 * result. `when` is the entry's rendered display stamp (the demo
 * histStamp's output on the fixed UTC+3 clock, like syncedAt); `path`
 * is the file-write target shown under the project (LOG-004, null on
 * entries that write no file); `code` is the `E_*` chip (null on
 * entries without one). Nulls instead of optional fields: every demo
 * row states all five facts.
 */
export type ActivityEntry = {
  readonly type: ActivityType
  readonly action: string
  readonly project: string | null
  readonly path: string | null
  readonly result: ActivityResult
  readonly code: string | null
  readonly when: string
}

/**
 * One typed block of a help article's body (the FAQ is Markdown in the
 * real app — HLP-005; the demo mini renderer's block vocabulary): a
 * section heading (the demo's `####`, an h4 under the pane's h3), a
 * paragraph, a fenced code block, or a list (`ordered` picks ol/ul).
 * Paragraph and list texts are plain strings — the demo's inline
 * Markdown emphasis (backticks, bold) is not modeled.
 */
export type HelpBlock =
  | { readonly kind: 'heading'; readonly text: string }
  | { readonly kind: 'paragraph'; readonly text: string }
  | { readonly kind: 'code'; readonly text: string }
  | { readonly kind: 'list'; readonly ordered: boolean; readonly items: readonly string[] }

/** One help article (HLP-001…005): the title and its typed body. */
export type HelpArticle = {
  readonly title: string
  readonly body: readonly HelpBlock[]
}

/** The §6.5 settings defaults (demo SETTINGS; the real app keeps them
 * in the settings table as JSON — D-18). Key names mirror the table. */
export type Settings = {
  readonly default: {
    readonly scheme: ProjectScheme
    readonly baseName: string
    readonly extraLocal: boolean
    readonly extraLocalPos: ExtraFilePosition
    readonly extraProject: boolean
    readonly extraProjectPos: ExtraFilePosition
  }
  readonly backup: { readonly retention: number }
  readonly sync: { readonly skipIdentical: boolean }
  readonly scan: {
    readonly maxDepth: number
    readonly excludeNames: readonly string[]
  }
  readonly history: { readonly maxRevisions: number }
  readonly log: { readonly maxEntries: number; readonly maxDays: number }
  readonly ui: { readonly language: string }
  readonly update: { readonly checkOnStart: boolean }
}
