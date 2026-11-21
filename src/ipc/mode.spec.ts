import { describe, expect, it } from 'vitest'
import { mockCommands } from './mock/commands'
import { mockEnabled } from './mode'

// The truth table the facade inlines (src/ipc/index.ts): the two spellings
// that enable mocks, and everything else keeping the real backend. The
// inline comparison in the facade must accept exactly these — its spec
// (index.spec.ts) pins the '1' half under the test environment.
describe('mockEnabled', () => {
  it.each(['1', 'true'])('enables the mock backend for %s', (raw) => {
    expect(mockEnabled(raw)).toBe(true)
  })

  it.each([undefined, '', '0', 'false', 'yes', 'TRUE'])('keeps the real backend for %s', (raw) => {
    expect(mockEnabled(raw)).toBe(false)
  })
})

describe('the mock command surface', () => {
  it('implements every command the generated bindings expose (D-108)', () => {
    // The §7.10/D-096 command names that exist in bindings.ts today: the
    // type annotation on `mockCommands` makes a missing entry a typecheck
    // failure; this pins the shipped set explicitly for the reader.
    expect(Object.keys(mockCommands).sort()).toEqual([
      'activityList',
      'appInfo',
      'appOpenDataFolder',
      'dbBackupsList',
      'dbRecreate',
      'dbRestoreBackup',
      'dialogPickFolder',
      'exclusionRemove',
      'exclusionsList',
      'markdownPreview',
      'projectAddManual',
      'projectsList',
      'projectsReorder',
      'settingsGet',
      'settingsSet',
    ])
  })
})
