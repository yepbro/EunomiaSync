/*
 * Seed exclusions (T-366): the frozen prototype's demo list
 * (docs/prototype/app.js EXCLUSIONS, ~4271) in the §6.2
 * `project_exclusions` DDL shape (path, reason, created stamp), typed
 * by ./types.ts. The demo's EXCLUSION_REASONS wording table is catalog
 * copy keyed by the reason (the badge text carries the meaning), not
 * seed data.
 */

import type { ExcludedPath } from './types'

/** The demo excluded paths, in the demo's order. */
export const exclusions: readonly ExcludedPath[] = [
  { path: '~/projects/archive/old-sandbox', reason: 'removed_scanned', addedAt: '5 Nov 2026' },
  { path: '~/projects/experiments/prototype-v0', reason: 'manual', addedAt: '1 Nov 2026' },
  { path: '~/vendor-legacy', reason: 'manual', addedAt: '20 Oct 2026' },
]
