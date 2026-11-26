/*
 * Seed sync (T-367): the frozen prototype's sync-summary demo
 * (docs/prototype/app.js SYNC_OUTCOMES ~3626 with the shared
 * SS_WRITTEN_REASON ~3622) and its pending list (MAIN-026), reduced to
 * data. The run itself (the tick timer, the per-tick column apply and
 * the dirty-badge clear) is the demo's script — JS is never ported
 * (D-206) — so the seed stores the run's END state: one row per
 * project in the matrix's column order with the demo's per-project
 * outcome. Target files are the demo targetPathOf's (~3700) rendered
 * display strings — "~" for the home prefix, the base file, scheme C
 * writing `<STEM>.project.md` (docktree's row) — stored explicitly,
 * like the projects' syncedAt stamps. The outcome badge and the four
 * counter chips are DERIVED by the dialog from these rows
 * (runOutcomeOf's rule: any skip or failure makes the run partial).
 *
 * Seed texts (reasons, codes, hints) are DATA, not UI copy: they render
 * as-is from the component; the status words, the counter labels and
 * the outcome note are the catalog's (closed vocabularies).
 */

import type { SyncPendingRow, SyncSummaryRow } from './types'

/** The run summary (OUT-009): every active project, column order. */
export const syncSummaryRows: readonly SyncSummaryRow[] = [
  {
    status: 'written',
    project: 'EunomiaSync',
    targetFile: '~/projects/business/EunomiaSync/AGENTS.md',
    reason: 'Pending assignments were applied — the target file was written.',
  },
  {
    status: 'unchanged',
    project: 'yepbro/site',
    targetFile: '~/projects/yepbro/site/AGENTS.md',
    reason: 'The built content matches the file, so it was not overwritten (D-24).',
  },
  {
    status: 'skipped',
    project: 'quotameter',
    targetFile: '~/projects/business/quotameter/CLAUDE.md',
    code: 'E_MARKERS_INVALID',
    reason:
      'Scheme B markers are missing or invalid in the target file — nothing was written (SYS-009).',
    hint: 'Paste the marker block with “Copy marker block” in the project settings.',
  },
  {
    status: 'written',
    project: 'task-id-mcp',
    targetFile: '~/projects/utils/task-id-mcp/AGENTS.md',
    reason: 'Pending assignments were applied — the target file was written.',
  },
  {
    status: 'skipped',
    project: 'utils/deja-vu',
    targetFile: '~/projects/utils/deja-vu/AGENTS.md',
    code: 'E_PROJECT_UNAVAILABLE',
    reason:
      'The project folder is unavailable (moved or the drive is disconnected) — the project was skipped (D-09).',
    hint: 'Use “Locate…” to set the new path, or archive the project.',
  },
  {
    status: 'written',
    project: 'docktree',
    targetFile: '~/projects/tools/docktree/AGENTS.project.md',
    reason: 'Pending assignments were applied — the target file was written.',
  },
  {
    status: 'written',
    project: 'moli',
    targetFile: '~/projects/apps/moli/AGENTS.md',
    reason: 'Pending assignments were applied — the target file was written.',
  },
  {
    status: 'failed',
    project: 'sandbox-cf',
    targetFile: '~/projects/labs/sandbox-cf/AGENTS.md',
    code: 'E_FILE_LOCKED',
    reason:
      'The target file is locked by another process — 3 write retries failed; the backup is kept (§10).',
    hint: 'Close the program that locks the file and run sync again.',
  },
]

/**
 * The pending list (MAIN-026): the demo's dirty columns (the seed
 * projects whose status is `dirty` — EunomiaSync, task-id-mcp,
 * docktree, moli) with their target files.
 */
export const syncPendingRows: readonly SyncPendingRow[] = [
  { project: 'EunomiaSync', targetFile: '~/projects/business/EunomiaSync/AGENTS.md' },
  { project: 'task-id-mcp', targetFile: '~/projects/utils/task-id-mcp/AGENTS.md' },
  { project: 'docktree', targetFile: '~/projects/tools/docktree/AGENTS.project.md' },
  { project: 'moli', targetFile: '~/projects/apps/moli/AGENTS.md' },
]
