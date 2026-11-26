/*
 * Seed settings (T-363): the §6.5 defaults of the frozen prototype
 * (docs/prototype/app.js `SETTINGS`, ~line 3880) typed by ./types.ts.
 * The real app keeps these in the settings table as JSON (D-18);
 * `sync.skip_identical` and `ui.language` have no control in the demo.
 */

import type { Settings } from './types'

export const settings: Settings = {
  default: {
    scheme: 'A',
    baseName: 'AGENTS.md',
    extraLocal: false,
    extraLocalPos: 'end',
    extraProject: false,
    extraProjectPos: 'end',
  },
  backup: { retention: 10 },
  sync: { skipIdentical: true },
  scan: {
    maxDepth: 6,
    excludeNames: ['node_modules', 'vendor', '.git', 'dist', 'build', 'target', '.venv'],
  },
  history: { maxRevisions: 200 },
  log: { maxEntries: 10000, maxDays: 180 },
  ui: { language: 'en' },
  update: { checkOnStart: true },
}
