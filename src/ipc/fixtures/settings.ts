// The thirteen §6.5 settings with their defaults, in spec table order —
// the exact list `settings_get` serves and the Settings window opens on
// (the D-052 wire rule: values travel as canonical JSON text; keys are the
// store's, not a frontend invention).

import type { SettingItem } from '../contract'

export const DEFAULT_SETTINGS: readonly SettingItem[] = [
  { key: 'default.scheme', value: '"A"' },
  { key: 'default.base_name', value: '"AGENTS.md"' },
  { key: 'default.extra_local', value: '{"enabled":false,"position":"end"}' },
  { key: 'default.extra_project', value: '{"enabled":false,"position":"end"}' },
  { key: 'backup.retention', value: '10' },
  { key: 'sync.skip_identical', value: 'true' },
  { key: 'scan.max_depth', value: '6' },
  {
    key: 'scan.exclude_names',
    value: '["node_modules","vendor",".git","dist","build","target",".venv"]',
  },
  { key: 'history.max_revisions', value: '200' },
  { key: 'log.max_entries', value: '10000' },
  { key: 'log.max_days', value: '180' },
  { key: 'ui.language', value: '"en"' },
  { key: 'update.check_on_start', value: 'true' },
]

/** The §6.5 keys, in table order. */
export const SETTING_KEYS: readonly string[] = DEFAULT_SETTINGS.map((s) => s.key)
