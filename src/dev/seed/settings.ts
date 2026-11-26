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

/*
 * The Settings window's number facts (T-367, DEF-001…004): the §6.5
 * closed ranges of the five number fields (demo SET_NUMBER_FIELDS,
 * docs/prototype/app.js ~3931) and the demo's one range-error state —
 * a typed value outside its range that the window renders with
 * aria-invalid and the inline error while the stored setting above
 * keeps the previous value (DEF-001: "the previous value stays until
 * it is fixed" — the stored 10, with 300 shown as the typed content).
 */

/** The five number fields the Settings window edits (DEF-001…004). */
export type SettingNumberField =
  'backupRetention' | 'scanDepth' | 'historyRevisions' | 'logEntries' | 'logDays'

/** A §6.5 closed range of one number field. */
export type SettingNumberRange = { readonly min: number; readonly max: number }

/** The §6.5 ranges, keyed per field (demo SET_NUMBER_FIELDS). */
export const settingNumberRanges: Record<SettingNumberField, SettingNumberRange> = {
  backupRetention: { min: 1, max: 200 },
  scanDepth: { min: 1, max: 20 },
  historyRevisions: { min: 10, max: 1000 },
  logEntries: { min: 1000, max: 100000 },
  logDays: { min: 7, max: 3650 },
}

/** The demo's range-error state: the field in error and the typed
 * out-of-range value its input shows. */
export const settingsRangeError: {
  readonly field: SettingNumberField
  readonly value: number
} = { field: 'backupRetention', value: 300 }
