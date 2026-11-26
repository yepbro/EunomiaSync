/*
 * Seed scan (T-366): the frozen prototype's scan demo
 * (docs/prototype/app.js ~3368–3430) reduced to data. The walk itself
 * (SCAN_WALK and its tick timer) is the demo's script — JS is never
 * ported (D-206) — so the seed stores the walk's END state: the
 * counters, the last current path, and the three result tables
 * (SCAN_NEW, SCAN_KNOWN, SCAN_SKIPPED). Row paths are the demo's
 * rendered display strings (root-prefixed), like the projects'
 * syncedAt stamps. The skip reasons carry the demo's five rows; the
 * spec's sixth reason (non-UTF-8) exists in the vocabulary
 * (./types.ts ScanSkipReason) with no demo row.
 */

import type { ScanKnownRow, ScanNewRow, ScanSkipRow } from './types'

/** The simulated root the demo scans (SCAN_ROOT). */
export const scanRoot = '~/projects'

/** The root's expanded form — the scan title's tooltip (the mock-up's
 * static markup fact, index.html 3222). */
export const scanRootPath = '/home/brooks/projects'

/** Folders checked at the walk's end (SCAN_WALK has 40 steps). */
export const scanFoldersChecked = 40

/** Repositories found at the walk's end: 8 known + 3 new + 1 archived. */
export const scanReposFound = 12

/** The current path at the walk's end (the walk's last step). */
export const scanCurrentPath = '~/projects/playground'

/** The "New" group (SCAN_NEW): repos the demo matrix does not have yet. */
export const scanNew: readonly ScanNewRow[] = [
  { name: 'notes-api', path: '~/projects/notes/notes-api' },
  { name: 'landing-new', path: '~/projects/yepbro/landing-new' },
  { name: 'scripts-kit', path: '~/projects/scripts/scripts-kit' },
]

/** The "Already added" group (SCAN_KNOWN): the demo projects again. */
export const scanKnown: readonly ScanKnownRow[] = [
  { name: 'EunomiaSync', path: '~/projects/business/EunomiaSync' },
  { name: 'yepbro/site', path: '~/projects/yepbro/site' },
  { name: 'quotameter', path: '~/projects/business/quotameter' },
  { name: 'task-id-mcp', path: '~/projects/utils/task-id-mcp' },
  { name: 'utils/deja-vu', path: '~/projects/utils/deja-vu' },
  { name: 'docktree', path: '~/projects/tools/docktree' },
  { name: 'moli', path: '~/projects/apps/moli' },
  { name: 'sandbox-cf', path: '~/projects/labs/sandbox-cf' },
]

/** The "Skipped" group (SCAN_SKIPPED): one row per demo reason. */
export const scanSkipped: readonly ScanSkipRow[] = [
  { path: '~/projects/business/quotameter/node_modules', reason: 'excluded' },
  { path: '~/projects/libs/web-grid', reason: 'submodule' },
  { path: '~/projects/private/locked-vault', reason: 'noAccess' },
  { path: '~/projects/scratch/old-playground', reason: 'inExclusions' },
  { path: '~/projects/attic/legacy-api', reason: 'archived' },
]
