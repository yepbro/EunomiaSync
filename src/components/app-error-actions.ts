// The "what to do" half of the UI-011 error alert (T-143): the §10
// user-action column, grouped. Spec §10 gives each ErrorCode a user
// action; twenty-one codes collapse onto seventeen action groups
// wherever the §10 actions genuinely coincide:
//
// - `fixTarget` — `E_TARGET_IS_DIR`/`E_TARGET_BROKEN_LINK`/
//   `E_TARGET_OUTSIDE_ROOT` (one §10 row, "fix the target");
// - `restoreFromBackup` — `E_WRITE_FAILED`/`E_VERIFY_FAILED` (one §10
//   row, "restore the file from the backup if needed");
// - `retry` — `E_DB_LOCKED` ("retry") and `E_UPDATE_FAILED` ("retry
//   later"). `E_SYNC_RUNNING` keeps its own `wait` group: §10's remedy
//   there ("wait until it finishes") presumes a running operation the
//   other two do not have (round-1 review P3-1).
//
// Every other code keeps its §10-specific action (`E_RESTORE_SOURCE_
// MISSING`'s "Pick another copy" included, BKP-012/T-353): merging
// further (say access-rights with disk-space) would blur concrete
// instructions the spec spells out.
//
// The group is keyed by `code`, not by `message_key`: §10's action
// column is per-code, the ErrorCode union is closed (a new code fails
// the `satisfies` check below until it joins a group), and message-
// family keys ride their code's group for free — keying by the open
// `message_key` string would need a lookup rule per family and could
// miss one.
//
// The one sanctioned exception is `ERROR_ACTION_OVERRIDES`: a family
// key whose §10 row is knowingly the wrong advice. Today exactly one —
// `errors.update_migration_failed` (UPD-004, produced by
// `DbInitError::app_error` in `db.rs`): a rolled-back migration has
// nothing "running" and nothing to retry into; the remedy is to
// install the previous version, and the producing code documents that
// "the UI shows the UPD-004 instruction, not the §10 row's prose".
// The override map's keys are open strings (family keys are an open
// vocabulary, D-034): a typo'd key simply never matches and the code's
// group applies — the pin test walks the values into the catalog.

import type { ErrorCode } from '../ipc'

/**
 * The §10 user-action groups. The value is the action group id; the
 * catalog key is `errors.actions.<id>`. `satisfies Record<ErrorCode,
 * string>` makes the map exhaustive over the closed ErrorCode union —
 * adding a code without a group is a compile error.
 */
export const ERROR_ACTION_GROUPS = {
  E_SYNC_RUNNING: 'wait',
  E_DB_LOCKED: 'retry',
  E_UPDATE_FAILED: 'retry',
  E_NOT_A_REPO: 'pickRepoRoot',
  E_PROJECT_UNAVAILABLE: 'relocate',
  E_NO_WRITE_ACCESS: 'fixAccess',
  E_TARGET_IS_DIR: 'fixTarget',
  E_TARGET_BROKEN_LINK: 'fixTarget',
  E_TARGET_OUTSIDE_ROOT: 'fixTarget',
  E_MARKERS_INVALID: 'pasteMarkerBlock',
  E_NOT_UTF8: 'reencode',
  E_BACKUP_FAILED: 'fixCauseAndRetry',
  E_DISK_FULL: 'freeSpace',
  E_FILE_LOCKED: 'closeAndRetry',
  E_WRITE_FAILED: 'restoreFromBackup',
  E_VERIFY_FAILED: 'restoreFromBackup',
  E_CONTENT_RESERVED: 'removeMarkerLine',
  E_VALIDATION: 'fixField',
  E_DB_CORRUPT: 'dbRecovery',
  E_DB_TOO_NEW: 'updateApp',
  E_RESTORE_SOURCE_MISSING: 'pickAnotherCopy',
} as const satisfies Record<ErrorCode, string>

/** An action group id; the catalog key is `errors.actions.<id>`. */
export type ErrorActionGroupId =
  | (typeof ERROR_ACTION_GROUPS)[ErrorCode]
  | (typeof ERROR_ACTION_OVERRIDES)[keyof typeof ERROR_ACTION_OVERRIDES]

/**
 * Family message keys whose §10 row is knowingly wrong advice, mapped
 * to their own action group (see the module docs for the UPD-004
 * case). Consulted before the code map, on an exact key match.
 */
export const ERROR_ACTION_OVERRIDES = {
  'errors.update_migration_failed': 'installPreviousVersion',
} as const

/**
 * The catalog key of the user action for an error: the family-key
 * override when one matches `messageKey` exactly, else the §10 group
 * of the code. Action messages may interpolate the error's `params`
 * (the override's does — the backup path).
 */
export function errorActionKey(
  code: ErrorCode,
  messageKey: string,
): `errors.actions.${ErrorActionGroupId}` {
  // Own-property lookup, not `in`: an inherited Object.prototype name
  // ("toString", …) must fall through to the code's group, not read a
  // function off the prototype chain and build a bogus catalog key.
  const overridden = Object.prototype.hasOwnProperty.call(ERROR_ACTION_OVERRIDES, messageKey)
    ? ERROR_ACTION_OVERRIDES[messageKey as keyof typeof ERROR_ACTION_OVERRIDES]
    : undefined
  if (overridden !== undefined) return `errors.actions.${overridden}`
  return `errors.actions.${ERROR_ACTION_GROUPS[code]}`
}
