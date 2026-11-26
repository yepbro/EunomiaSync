/*
 * Seed archived projects (T-366): the frozen prototype's demo list
 * (docs/prototype/app.js ARCHIVED, ~4164) typed by ./types.ts. The
 * demo stores `addedVia` as display wording; the seed carries the
 * origin value instead and the rendering maps it to the same two
 * wordings the settings modal's "Added" row uses.
 */

import type { ArchivedProject } from './types'

/** The demo archived projects, in the demo's order. */
export const archivedProjects: readonly ArchivedProject[] = [
  {
    name: 'old-crm',
    path: '~/projects/legacy/old-crm',
    color: '#8b5cf6',
    archivedAt: '3 Nov 2026',
    origin: 'scan',
  },
  {
    name: 'legacy-parser',
    path: '~/projects/legacy/legacy-parser',
    color: '#f59e0b',
    archivedAt: '28 Oct 2026',
    origin: 'manual',
  },
  {
    name: 'wiki-bot',
    path: '~/projects/apps/wiki-bot',
    color: '#06b6d4',
    archivedAt: '12 Oct 2026',
    origin: 'scan',
  },
]
