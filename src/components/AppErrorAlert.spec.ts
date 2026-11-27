import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import type { VueWrapper } from '@vue/test-utils'
import { nextTick } from 'vue'
import AppErrorAlert from './AppErrorAlert.vue'
import { ERROR_ACTION_GROUPS, ERROR_ACTION_OVERRIDES, errorActionKey } from './app-error-actions'
import { i18n } from '../i18n'
import type { AppErrorWire, ErrorCode } from '../ipc'

// The UI-011 criterion, behaviorally: an error carrying `params` and
// `details` renders all four blocks (what/where/why/what-to-do) plus
// the collapsed copyable envelope; unknown message keys fall back
// honestly; the OS facts stay visible (BKP-005) while the raw OS
// message stays in the envelope. Every assertion derives from the
// spec/copy text, never from the component's internals.

/** A §7.10 envelope with every block exercised (params + OS details). */
const writeFailed: AppErrorWire = {
  code: 'E_WRITE_FAILED',
  message_key: 'errors.write_failed',
  params: { project: 'web', path: '/srv/web/AGENTS.md' },
  details: {
    os_code: 13,
    os_kind: 'PermissionDenied',
    os_message: 'Permission denied (os error 13)',
  },
}

/** The exact JSON the Copy button must put on the clipboard. */
const WRITE_FAILED_ENVELOPE = JSON.stringify(
  {
    code: 'E_WRITE_FAILED',
    message_key: 'errors.write_failed',
    params: { project: 'web', path: '/srv/web/AGENTS.md' },
    details: {
      os_code: 13,
      os_kind: 'PermissionDenied',
      os_message: 'Permission denied (os error 13)',
    },
  },
  null,
  2,
)

function mountAlert(error: AppErrorWire, contextKey?: string): VueWrapper {
  return mount(AppErrorAlert, {
    props: contextKey === undefined ? { error } : { error, contextKey },
    global: { plugins: [i18n] },
  })
}

/** The rendered text of the labeled row `<dt>Label</dt><dd>value</dd>`. */
function rowValue(wrapper: VueWrapper, label: string): string {
  const dts = wrapper.findAll('dt')
  const index = dts.findIndex((dt) => dt.text() === label)
  if (index === -1) throw new Error(`no row labeled "${label}"`)
  return wrapper.findAll('dd')[index].text()
}

describe('AppErrorAlert', () => {
  describe('UI-011 blocks for an error with params and details', () => {
    let wrapper: VueWrapper
    beforeEach(() => {
      wrapper = mountAlert(writeFailed)
    })

    it('announces itself as an alert', () => {
      expect(wrapper.find('[role="alert"]').exists()).toBe(true)
    })

    it('keeps the disclosure and the copy feedback outside the alert region', () => {
      // The alert announces the error; the collapsed envelope, its
      // Copy button and the feedback live region are tools beside it
      // (nested live regions re-announce unpredictably).
      const alert = wrapper.find('[role="alert"]')
      expect(alert.find('details').exists()).toBe(false)
      expect(alert.find('.copy').exists()).toBe(false)
      expect(alert.find('[role="status"]').exists()).toBe(false)
    })

    it('renders the "what happened" title from the message key', () => {
      expect(wrapper.find('.title').text()).toBe('⚠ Writing the target file failed')
    })

    it('renders the "where" row from the path and project params', () => {
      expect(rowValue(wrapper, 'Where')).toBe('web — /srv/web/AGENTS.md')
    })

    it('renders the "why" row with the code and the visible OS facts', () => {
      // BKP-005: the OS code and error type are visible, not collapsed.
      expect(rowValue(wrapper, 'Why')).toBe('E_WRITE_FAILED — OS error 13 (PermissionDenied)')
    })

    it('renders the "what to do" row with the §10 user action', () => {
      expect(rowValue(wrapper, 'What to do')).toBe(
        'Run the sync again; if the file stays damaged, restore it from the backup in the data directory.',
      )
    })

    it('keeps the raw OS message out of the visible rows (envelope only)', () => {
      expect(wrapper.find('.facts').text()).not.toContain('Permission denied (os error 13)')
    })

    it('collapses the full envelope behind the details disclosure', () => {
      const details = wrapper.find('details')
      expect(details.exists()).toBe(true)
      expect((details.element as HTMLDetailsElement).open).toBe(false)
      expect(wrapper.find('summary').text()).toBe('Technical details')
    })

    it('renders code, message_key, params and details inside the envelope', () => {
      // The card's criterion: an error with params and details renders
      // them — the envelope carries the full §7.10 wire form.
      expect(wrapper.find('.envelope').text()).toBe(WRITE_FAILED_ENVELOPE)
    })
  })

  describe('title interpolation of params (producer-stable keys)', () => {
    it('interpolates the schema versions of a too-new database', () => {
      // The shape `DbInitError::app_error` pins (db.rs TooNew arm).
      const error: AppErrorWire = {
        code: 'E_DB_TOO_NEW',
        message_key: 'errors.db_too_new',
        params: {
          path: '/data/db/eunomiasync.sqlite3',
          database_schema_version: '2',
          app_schema_version: '1',
        },
        details: null,
      }
      const wrapper = mountAlert(error)
      expect(wrapper.find('.title').text()).toBe(
        '⚠ The database uses schema 2, but this app supports only 1',
      )
    })

    it('interpolates the quick_check evidence of a corrupt database', () => {
      const error: AppErrorWire = {
        code: 'E_DB_CORRUPT',
        message_key: 'errors.db_corrupt',
        params: { path: '/data/db/eunomiasync.sqlite3', detail: 'row 3 missing data' },
        details: null,
      }
      const wrapper = mountAlert(error)
      expect(wrapper.find('.title').text()).toBe('⚠ The database is damaged (row 3 missing data)')
    })
  })

  describe('the UPD-004 family key (rolled-back migration)', () => {
    // `errors.update_migration_failed` is the one non-default key the
    // Rust side already emits (db.rs Migration arm, params pinned by
    // the db suite): its title and its action override the §10 row.
    const migrationFailed: AppErrorWire = {
      code: 'E_UPDATE_FAILED',
      message_key: 'errors.update_migration_failed',
      params: {
        path: '/data/db/eunomiasync.sqlite3',
        backup_path: '/data/db-backups/pre-migration-v1-20261118T101500123Z.sqlite3',
      },
      details: null,
    }

    it('renders the UPD-004 title', () => {
      const wrapper = mountAlert(migrationFailed)
      expect(wrapper.find('.title').text()).toBe(
        '⚠ A database migration failed and was rolled back; the app did not change the database',
      )
    })

    it('overrides the §10 action with the install-previous-version advice', () => {
      const wrapper = mountAlert(migrationFailed)
      expect(rowValue(wrapper, 'What to do')).toBe(
        'Install the previous app version, or update to a fixed release. The pre-migration copy of the database is at /data/db-backups/pre-migration-v1-20261118T101500123Z.sqlite3.',
      )
    })
  })

  describe('fallback for an unknown message key', () => {
    it('falls back to the generic title while the other blocks keep rendering', () => {
      // `errors.validation.name_too_long` is the documented
      // `E_VALIDATION` family shape (errors.rs's message-family
      // invariant) — the catalog does not carry family members yet, so
      // this is the honest drift case.
      const error: AppErrorWire = {
        ...writeFailed,
        code: 'E_VALIDATION',
        message_key: 'errors.validation.name_too_long',
      }
      const wrapper = mountAlert(error)
      expect(wrapper.find('.title').text()).toBe('⚠ Something went wrong')
      expect(rowValue(wrapper, 'Where')).toBe('web — /srv/web/AGENTS.md')
      expect(rowValue(wrapper, 'Why')).toBe('E_VALIDATION — OS error 13 (PermissionDenied)')
      // The family key rides its code's §10 action group.
      expect(rowValue(wrapper, 'What to do')).toBe('Fix the invalid field, then save again.')
      expect(wrapper.find('.envelope').text()).toContain(
        '"message_key": "errors.validation.name_too_long"',
      )
    })
  })

  describe('the "where" row tracks the params that exist', () => {
    it.each([
      ['path only', { path: '/srv/web' }, '/srv/web'],
      ['project only', { project: 'web' }, 'web'],
    ])('renders %s', (_name, params, expected) => {
      const wrapper = mountAlert({ ...writeFailed, params })
      expect(rowValue(wrapper, 'Where')).toBe(expected)
    })

    it('omits the row when params carry neither path nor project', () => {
      const wrapper = mountAlert({
        ...writeFailed,
        params: { field: 'name' },
      })
      expect(wrapper.findAll('dt').map((dt) => dt.text())).not.toContain('Where')
    })
  })

  describe('the "why" row tracks the OS facts that exist', () => {
    it('shows the code alone when there are no details', () => {
      const wrapper = mountAlert({ ...writeFailed, details: null })
      expect(rowValue(wrapper, 'Why')).toBe('E_WRITE_FAILED')
    })

    it('shows the OS code without a kind', () => {
      const wrapper = mountAlert({
        ...writeFailed,
        details: { os_code: 28, os_kind: null, os_message: null },
      })
      expect(rowValue(wrapper, 'Why')).toBe('E_WRITE_FAILED — OS error 28')
    })

    it('shows the kind when the OS code is absent (BKP-005 "or error type")', () => {
      const wrapper = mountAlert({
        ...writeFailed,
        details: { os_code: null, os_kind: 'DatabaseBusy', os_message: 'database is locked' },
      })
      expect(rowValue(wrapper, 'Why')).toBe('E_WRITE_FAILED — DatabaseBusy')
    })
  })

  describe('§10 action mapping (rendered behavior)', () => {
    const actionFor = (code: ErrorCode, messageKey: string): string =>
      rowValue(
        mountAlert({ ...writeFailed, code, message_key: messageKey, details: null }),
        'What to do',
      )

    it('shares one action between E_WRITE_FAILED and E_VERIFY_FAILED', () => {
      const write = actionFor('E_WRITE_FAILED', 'errors.write_failed')
      const verify = actionFor('E_VERIFY_FAILED', 'errors.verify_failed')
      expect(write).toBe(verify)
      expect(write).toBe(
        'Run the sync again; if the file stays damaged, restore it from the backup in the data directory.',
      )
    })

    it('shares one action between the three E_TARGET_* codes', () => {
      const actions = [
        actionFor('E_TARGET_IS_DIR', 'errors.target_is_dir'),
        actionFor('E_TARGET_BROKEN_LINK', 'errors.target_broken_link'),
        actionFor('E_TARGET_OUTSIDE_ROOT', 'errors.target_outside_root'),
      ]
      expect(new Set(actions).size).toBe(1)
      expect(actions[0]).toBe(
        'Fix the target path so it is a regular file inside the project root.',
      )
    })

    it('keeps "wait" for the running sync apart from plain "retry"', () => {
      // §10's remedies differ: E_SYNC_RUNNING presumes a running
      // operation; the busy-db and update failures do not.
      expect(actionFor('E_SYNC_RUNNING', 'errors.sync_running')).toBe(
        'Wait until the running operation finishes, then try again.',
      )
      expect(actionFor('E_DB_LOCKED', 'errors.db_locked')).toBe('Try again in a moment.')
      expect(actionFor('E_UPDATE_FAILED', 'errors.update_failed')).toBe('Try again in a moment.')
    })
  })

  describe('operation context', () => {
    it('renders the context line for a known key', () => {
      const wrapper = mountAlert(writeFailed, 'app.title')
      expect(wrapper.find('.context').text()).toBe('EunomiaSync')
    })

    it('renders nothing for an unknown key', () => {
      const wrapper = mountAlert(writeFailed, 'errors.no.such.key')
      expect(wrapper.find('.context').exists()).toBe(false)
    })

    it('renders nothing without a context key', () => {
      expect(mountAlert(writeFailed).find('.context').exists()).toBe(false)
    })
  })

  describe('copying the envelope', () => {
    const writeText = vi.fn<(text: string) => Promise<void>>()
    let restoreExecCommand: (() => void) | undefined

    /** Installs a mock `document.execCommand`; afterEach restores it. */
    function mockExecCommand(implementation: () => boolean): () => boolean {
      const hadOwn = Object.prototype.hasOwnProperty.call(document, 'execCommand')
      const previous = document.execCommand
      document.execCommand = implementation as typeof document.execCommand
      restoreExecCommand = () => {
        if (hadOwn) document.execCommand = previous as typeof document.execCommand
        else delete (document as { execCommand?: unknown }).execCommand
      }
      return implementation
    }

    beforeEach(() => {
      writeText.mockReset().mockResolvedValue(undefined)
      vi.stubGlobal('navigator', { clipboard: { writeText } })
    })

    afterEach(() => {
      restoreExecCommand?.()
      restoreExecCommand = undefined
      vi.unstubAllGlobals()
      vi.useRealTimers()
    })

    it('puts the full envelope on the clipboard and confirms', async () => {
      const wrapper = mountAlert(writeFailed)
      await wrapper.find('.copy').trigger('click')
      await flushPromises()
      expect(writeText).toHaveBeenCalledExactlyOnceWith(WRITE_FAILED_ENVELOPE)
      expect(wrapper.find('.copy').text()).toBe('Copied')
      expect(wrapper.find('[role="status"]').text()).toBe('Error details copied to the clipboard.')
    })

    it('reverts the confirmation after a short delay', async () => {
      vi.useFakeTimers()
      const wrapper = mountAlert(writeFailed)
      const clicked = wrapper.find('.copy').trigger('click')
      await vi.advanceTimersByTimeAsync(0)
      await clicked
      expect(wrapper.find('.copy').text()).toBe('Copied')
      vi.advanceTimersByTime(2500)
      await nextTick()
      expect(wrapper.find('.copy').text()).toBe('Copy')
    })

    it('falls back to the legacy copy path when the clipboard API is missing', async () => {
      vi.stubGlobal('navigator', { clipboard: undefined })
      let textareaValue: string | undefined
      const execCommand = mockExecCommand(
        vi.fn(() => {
          textareaValue = document.querySelector('textarea')?.value
          return true
        }),
      )
      const wrapper = mountAlert(writeFailed)
      await wrapper.find('.copy').trigger('click')
      await flushPromises()
      // The fallback must copy the same envelope, not just run.
      expect(execCommand).toHaveBeenCalledWith('copy')
      expect(textareaValue).toBe(WRITE_FAILED_ENVELOPE)
      expect(document.querySelector('textarea')).toBeNull()
      expect(wrapper.find('.copy').text()).toBe('Copied')
    })

    it('falls back to the legacy path when writeText rejects', async () => {
      writeText.mockRejectedValueOnce(new Error('denied'))
      let textareaValue: string | undefined
      const execCommand = mockExecCommand(
        vi.fn(() => {
          textareaValue = document.querySelector('textarea')?.value
          return true
        }),
      )
      const wrapper = mountAlert(writeFailed)
      await wrapper.find('.copy').trigger('click')
      await flushPromises()
      expect(execCommand).toHaveBeenCalledOnce()
      expect(textareaValue).toBe(WRITE_FAILED_ENVELOPE)
      expect(wrapper.find('.copy').text()).toBe('Copied')
    })

    it('shows a failed state (label + announcement) when every path fails', async () => {
      writeText.mockRejectedValueOnce(new Error('denied'))
      mockExecCommand(vi.fn(() => false))
      const wrapper = mountAlert(writeFailed)
      await wrapper.find('.copy').trigger('click')
      await flushPromises()
      expect(wrapper.find('.copy').text()).toBe('Copy failed')
      expect(wrapper.find('[role="status"]').text()).toBe('Copying the error details failed.')
    })

    it('shows the failed state when the legacy path throws (no unhandled rejection)', async () => {
      vi.useFakeTimers()
      vi.stubGlobal('navigator', { clipboard: undefined })
      mockExecCommand(
        vi.fn(() => {
          throw new Error('SecurityError')
        }),
      )
      const wrapper = mountAlert(writeFailed)
      const clicked = wrapper.find('.copy').trigger('click')
      await vi.advanceTimersByTimeAsync(0)
      await clicked
      expect(wrapper.find('.copy').text()).toBe('Copy failed')
      expect(wrapper.find('[role="status"]').text()).toBe('Copying the error details failed.')
      // The failed feedback reverts like the copied one.
      vi.advanceTimersByTime(2500)
      await nextTick()
      expect(wrapper.find('.copy').text()).toBe('Copy')
      expect(wrapper.find('[role="status"]').text()).toBe('')
    })
  })
})

// The catalog side of the contract, mirrored from the spec §10 table
// (the same derivation errors.rs's own pinned registry test uses): the
// default message keys, the code→action-group table, and the one
// family-key override must all resolve — a new code or a remapped
// action fails here.

/** The twenty default `errors.<snake_case>` keys (`ErrorCode::message_key`). */
const DEFAULT_MESSAGE_KEYS = {
  E_SYNC_RUNNING: 'errors.sync_running',
  E_NOT_A_REPO: 'errors.not_a_repo',
  E_PROJECT_UNAVAILABLE: 'errors.project_unavailable',
  E_NO_WRITE_ACCESS: 'errors.no_write_access',
  E_TARGET_IS_DIR: 'errors.target_is_dir',
  E_TARGET_BROKEN_LINK: 'errors.target_broken_link',
  E_TARGET_OUTSIDE_ROOT: 'errors.target_outside_root',
  E_MARKERS_INVALID: 'errors.markers_invalid',
  E_NOT_UTF8: 'errors.not_utf8',
  E_BACKUP_FAILED: 'errors.backup_failed',
  E_DISK_FULL: 'errors.disk_full',
  E_FILE_LOCKED: 'errors.file_locked',
  E_WRITE_FAILED: 'errors.write_failed',
  E_VERIFY_FAILED: 'errors.verify_failed',
  E_CONTENT_RESERVED: 'errors.content_reserved',
  E_VALIDATION: 'errors.validation',
  E_DB_LOCKED: 'errors.db_locked',
  E_DB_CORRUPT: 'errors.db_corrupt',
  E_DB_TOO_NEW: 'errors.db_too_new',
  E_RESTORE_SOURCE_MISSING: 'errors.restore_source_missing',
  E_UPDATE_FAILED: 'errors.update_failed',
} as const satisfies Record<ErrorCode, string>

/** The §10 user-action column as code → action group. */
const CODE_ACTIONS: Readonly<Record<ErrorCode, string>> = {
  E_SYNC_RUNNING: 'wait',
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
  E_DB_LOCKED: 'retry',
  E_DB_CORRUPT: 'dbRecovery',
  E_DB_TOO_NEW: 'updateApp',
  E_RESTORE_SOURCE_MISSING: 'pickAnotherCopy',
  E_UPDATE_FAILED: 'retry',
}

describe('errors catalog (en)', () => {
  const { t, te } = i18n.global

  it('resolves every registered default message key', () => {
    const missing = Object.values(DEFAULT_MESSAGE_KEYS).filter((key) => !te(key))
    expect(missing).toEqual([])
    // And they are real copy, not leaked key strings.
    expect(t('errors.write_failed')).toBe('Writing the target file failed')
  })

  it('maps every MVP code to its §10 action group (default key, no override)', () => {
    for (const [code, group] of Object.entries(CODE_ACTIONS)) {
      expect(errorActionKey(code as ErrorCode, DEFAULT_MESSAGE_KEYS[code as ErrorCode])).toBe(
        `errors.actions.${group}`,
      )
    }
  })

  it('overrides the §10 action for the UPD-004 family key only', () => {
    expect(errorActionKey('E_UPDATE_FAILED', 'errors.update_migration_failed')).toBe(
      'errors.actions.installPreviousVersion',
    )
    // The code's default key keeps the §10 group.
    expect(errorActionKey('E_UPDATE_FAILED', 'errors.update_failed')).toBe('errors.actions.retry')
    expect(ERROR_ACTION_OVERRIDES).toEqual({
      'errors.update_migration_failed': 'installPreviousVersion',
    })
  })

  it('ignores prototype-named message keys (own-property lookup only)', () => {
    // "toString" is inherited from Object.prototype: the override
    // lookup must miss it and fall back to the code's §10 group.
    expect(errorActionKey('E_WRITE_FAILED', 'toString')).toBe('errors.actions.restoreFromBackup')
  })

  it('resolves an action message for every §10 action group', () => {
    const groups = new Set([
      ...Object.values(ERROR_ACTION_GROUPS),
      ...Object.values(ERROR_ACTION_OVERRIDES),
    ])
    const missing = [...groups].filter((group) => !te(`errors.actions.${group}`))
    expect(missing).toEqual([])
  })
})
