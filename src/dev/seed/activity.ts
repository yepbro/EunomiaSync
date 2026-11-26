/*
 * Seed activity (T-367): the demo action log of the frozen prototype
 * (docs/prototype/app.js ACTIVITY ~4370, ACT_TYPES ~4530), reduced to
 * data. The four filters and their combining (renderActivity's query)
 * are behavior — JS is never ported (D-206) — so the seed stores the
 * demo log in its RENDERED order: renderActivity sorts the demo
 * offsets ascending (app.js ~4578), which is NEWEST FIRST — within
 * each run the finish line precedes the start line (the demo ARRAY's
 * narrative order is not kept). The dialog renders the rows in prop
 * order. Stamps are the demo histStamp's output per offset on the
 * fixed UTC+3 demo clock ("now" = 19 Nov 2026, 14:32 UTC+3), the
 * history panel's precedent; the demo's `minutesBack` number is not
 * carried. The demo's `file: true` flag becomes the rendered `path`
 * (app.js targetPathOf of the entry's project, LOG-004) — display
 * strings stay data. Nulls state the absences explicitly (no project,
 * no file write, no code).
 *
 * Seed texts (actions) are DATA, not UI copy. The type and result
 * words are the catalog's (closed vocabularies — the chip labels and
 * the Type/Result filter options).
 */

import { projects } from './projects'
import { settings } from './settings'
import type { ActivityEntry } from './types'

/** The demo entries, newest first (renderActivity's sort order). */
export const activityEntries: readonly ActivityEntry[] = [
  /* The sync run of today, 14:32 — the one the sync bar counts as the
     last successful sync (LOG-007); finish → write → backup → start. */
  {
    type: 'sync',
    action: 'Sync run finished — status: success',
    project: null,
    path: null,
    result: 'ok',
    code: null,
    when: '19 Nov 2026, 14:32 (UTC+3)',
  },
  {
    type: 'sync',
    action: 'Target file written',
    project: 'EunomiaSync',
    path: '~/projects/business/EunomiaSync/AGENTS.md',
    result: 'ok',
    code: null,
    when: '19 Nov 2026, 14:31 (UTC+3)',
  },
  {
    type: 'backups',
    action: 'Backup created — AGENTS.md',
    project: 'EunomiaSync',
    path: '~/projects/business/EunomiaSync/AGENTS.md',
    result: 'ok',
    code: null,
    when: '19 Nov 2026, 14:30 (UTC+3)',
  },
  {
    type: 'sync',
    action: 'Sync run started',
    project: null,
    path: null,
    result: 'ok',
    code: null,
    when: '19 Nov 2026, 14:29 (UTC+3)',
  },
  /* Assignments: bulk ops log one entry with the change count (LOG-004). */
  {
    type: 'assignments',
    action: 'Bulk assign — 12 assignments changed (“Naming conventions”)',
    project: null,
    path: null,
    result: 'ok',
    code: null,
    when: '19 Nov 2026, 13:47 (UTC+3)',
  },
  {
    type: 'assignments',
    action: 'Assigned variant “Conventional Commits” to “Commit messages”',
    project: 'docktree',
    path: null,
    result: 'ok',
    code: null,
    when: '19 Nov 2026, 11:12 (UTC+3)',
  },
  /* The sync run of yesterday (finished 20:43) — partial: skips and
     failures do not stop it (D-09); per-project results carry the
     sync summary codes. Finish first, then the writes newest-first. */
  {
    type: 'sync',
    action: 'Sync run finished — status: partial (2 skipped, 1 failed)',
    project: null,
    path: null,
    result: 'ok',
    code: null,
    when: '18 Nov 2026, 20:43 (UTC+3)',
  },
  {
    type: 'sync',
    action: 'Target file written',
    project: 'task-id-mcp',
    path: '~/projects/utils/task-id-mcp/AGENTS.md',
    result: 'ok',
    code: null,
    when: '18 Nov 2026, 20:42 (UTC+3)',
  },
  {
    type: 'sync',
    action: 'File write failed',
    project: 'sandbox-cf',
    path: '~/projects/labs/sandbox-cf/AGENTS.md',
    result: 'error',
    code: 'E_FILE_LOCKED',
    when: '18 Nov 2026, 20:41 (UTC+3)',
  },
  {
    type: 'sync',
    action: 'File write skipped',
    project: 'utils/deja-vu',
    path: '~/projects/utils/deja-vu/AGENTS.md',
    result: 'skipped',
    code: 'E_PROJECT_UNAVAILABLE',
    when: '18 Nov 2026, 20:40 (UTC+3)',
  },
  {
    type: 'sync',
    action: 'File write skipped',
    project: 'quotameter',
    path: '~/projects/business/quotameter/CLAUDE.md',
    result: 'skipped',
    code: 'E_MARKERS_INVALID',
    when: '18 Nov 2026, 20:39 (UTC+3)',
  },
  /* Library: instructions and variants, headings. */
  {
    type: 'library',
    action: 'Added variant “Bug-first” to “Bug-driven development”',
    project: null,
    path: null,
    result: 'ok',
    code: null,
    when: '18 Nov 2026, 12:40 (UTC+3)',
  },
  {
    type: 'library',
    action: 'Edited instruction “Dependency audit”',
    project: null,
    path: null,
    result: 'ok',
    code: null,
    when: '17 Nov 2026, 16:20 (UTC+3)',
  },
  {
    type: 'library',
    action: 'Renamed heading “Git and branches” → “Git Workflow”',
    project: null,
    path: null,
    result: 'ok',
    code: null,
    when: '16 Nov 2026, 11:00 (UTC+3)',
  },
  {
    type: 'library',
    action: 'Moved heading “Communication” under “General”',
    project: null,
    path: null,
    result: 'ok',
    code: null,
    when: '15 Nov 2026, 09:30 (UTC+3)',
  },
  {
    type: 'library',
    action: 'Deleted variant “Old wording” of “Commit messages”',
    project: null,
    path: null,
    result: 'ok',
    code: null,
    when: '14 Nov 2026, 15:45 (UTC+3)',
  },
  {
    type: 'library',
    action: 'Restored instruction “Secrets handling”',
    project: null,
    path: null,
    result: 'ok',
    code: null,
    when: '13 Nov 2026, 18:30 (UTC+3)',
  },
  {
    type: 'library',
    action: 'Hid instruction “Secrets handling”',
    project: null,
    path: null,
    result: 'ok',
    code: null,
    when: '13 Nov 2026, 10:12 (UTC+3)',
  },
  /* Projects: added (manual / scan), settings, archived, restored. */
  {
    type: 'projects',
    action: 'Added project moli (manual)',
    project: 'moli',
    path: null,
    result: 'ok',
    code: null,
    when: '12 Nov 2026, 16:00 (UTC+3)',
  },
  {
    type: 'projects',
    action: 'Added project sandbox-cf (found by scan)',
    project: 'sandbox-cf',
    path: null,
    result: 'ok',
    code: null,
    when: '11 Nov 2026, 19:25 (UTC+3)',
  },
  {
    type: 'projects',
    action: 'Changed project settings — quotameter (target file CLAUDE.md)',
    project: 'quotameter',
    path: null,
    result: 'ok',
    code: null,
    when: '11 Nov 2026, 06:20 (UTC+3)',
  },
  {
    type: 'projects',
    action: 'Restored project old-site',
    project: 'old-site',
    path: null,
    result: 'ok',
    code: null,
    when: '9 Nov 2026, 15:40 (UTC+3)',
  },
  {
    type: 'projects',
    action: 'Archived project old-crm',
    project: 'old-crm',
    path: null,
    result: 'ok',
    code: null,
    when: '3 Nov 2026, 09:12 (UTC+3)',
  },
  {
    type: 'library',
    action: 'Created instruction “Dependency audit”',
    project: null,
    path: null,
    result: 'ok',
    code: null,
    when: '2 Nov 2026, 10:00 (UTC+3)',
  },
]

/** The Project filter's options (the demo mirrors the toolbar's demo
 * projects — the seed's column order). */
export const activityProjects: readonly string[] = projects.map((project) => project.name)

/** The §7.7 UI row's log-limits hint numbers (LOG-006): the settings
 * seed's ACTIVE limits — derived, not repeated (a future settings
 * change moves the hint with it). */
export const activityLogLimits = {
  entries: settings.log.maxEntries,
  days: settings.log.maxDays,
}
