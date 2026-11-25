import { beforeEach, describe, expect, it, vi } from 'vitest'
import type {
  ProjectAvailabilityEventWire,
  ScanProgressEventWire,
  SyncProgressEventWire,
} from '../contract'
import type { FixtureProfile } from '../fixtures/generator'
import { mockCommands } from './commands'
import {
  applyMockAvailability,
  mockState,
  resetMockState,
  type MockUnavailableReason,
} from './state'
import { busEvent, createEventBus, mockEventBus, mockEvents } from './events'

/** A test-local payload map for the bus-mechanics tests: locally-typed
 * payloads, deliberately NOT the generated wire shapes (`scan://progress`
 * gained its real generated payload in `IpcEventMap` with T-076; these
 * fixtures keep testing the bus, not the event). */
interface TestEvents extends Record<string, unknown> {
  'scan://progress': { done: number; total: number }
  'state://dirty': { projectIds: string[] }
}

describe('createEventBus', () => {
  it('delivers an emitted payload to the subscriber', () => {
    const bus = createEventBus<TestEvents>()
    const listener = vi.fn()
    bus.on('scan://progress', listener)

    bus.emit('scan://progress', { done: 3, total: 10 })

    expect(listener).toHaveBeenCalledTimes(1)
    expect(listener).toHaveBeenCalledWith({ done: 3, total: 10 })
  })

  it('delivers to several listeners in subscription order', () => {
    const bus = createEventBus<TestEvents>()
    const calls: string[] = []
    bus.on('state://dirty', () => calls.push('first'))
    bus.on('state://dirty', () => calls.push('second'))

    bus.emit('state://dirty', { projectIds: ['p1'] })

    expect(calls).toEqual(['first', 'second'])
  })

  it('stops delivery after unsubscribe, and only for that listener', () => {
    const bus = createEventBus<TestEvents>()
    const gone = vi.fn()
    const kept = vi.fn()
    const unsubscribe = bus.on('scan://progress', gone)
    bus.on('scan://progress', kept)
    expect(bus.listenerCount()).toBe(2)

    unsubscribe()
    bus.emit('scan://progress', { done: 1, total: 2 })

    expect(gone).not.toHaveBeenCalled()
    expect(kept).toHaveBeenCalledTimes(1)
    expect(kept).toHaveBeenCalledWith({ done: 1, total: 2 })
    expect(bus.listenerCount()).toBe(1)
  })

  it('keeps event names apart', () => {
    const bus = createEventBus<TestEvents>()
    const progress = vi.fn()
    const dirty = vi.fn()
    bus.on('scan://progress', progress)
    bus.on('state://dirty', dirty)

    bus.emit('state://dirty', { projectIds: [] })

    expect(progress).not.toHaveBeenCalled()
    expect(dirty).toHaveBeenCalledTimes(1)
  })

  it('keeps emitting to a listener unsubscribed mid-delivery (copied set)', () => {
    const bus = createEventBus<TestEvents>()
    const late = vi.fn()
    let unsubLate: () => void = () => {}
    // `early` is subscribed FIRST so it runs first and unsubscribes
    // `late` before `late`'s turn in the same emission.
    const early = vi.fn(() => unsubLate())
    bus.on('scan://progress', early)
    unsubLate = bus.on('scan://progress', late)

    bus.emit('scan://progress', { done: 1, total: 4 })
    // The snapshot semantics: the current emission still reaches `late`.
    expect(early).toHaveBeenCalledTimes(1)
    expect(late).toHaveBeenCalledTimes(1)

    bus.emit('scan://progress', { done: 2, total: 4 })
    expect(early).toHaveBeenCalledTimes(2)
    expect(late).toHaveBeenCalledTimes(1)
  })

  it('emitting without listeners is a no-op', () => {
    const bus = createEventBus<TestEvents>()
    expect(() => bus.emit('scan://progress', { done: 0, total: 0 })).not.toThrow()
    expect(bus.listenerCount()).toBe(0)
  })
})

describe('mockEventBus', () => {
  it('delivers a typed project-availability payload and stops after unsubscribe', () => {
    const listener = vi.fn()
    const unsubscribe = mockEventBus.on('state://project-availability', listener)
    const payload: ProjectAvailabilityEventWire = {
      changes: [{ project_id: 'p1', available: false, unavailable_reason: 'missing' }],
    }
    try {
      mockEventBus.emit('state://project-availability', payload)

      expect(listener).toHaveBeenCalledTimes(1)
      expect(listener).toHaveBeenCalledWith(payload)
    } finally {
      unsubscribe()
    }

    mockEventBus.emit('state://project-availability', payload)

    expect(listener).toHaveBeenCalledTimes(1)
    expect(mockEventBus.listenerCount()).toBe(0)
  })
})

describe('mockEvents', () => {
  it('stateProjectAvailability carries the generated payload type over the bus', () => {
    const listener = vi.fn()
    const payload: ProjectAvailabilityEventWire = {
      changes: [{ project_id: 'p1', available: true, unavailable_reason: null }],
    }
    const stop = mockEvents.stateProjectAvailability.on(listener)
    try {
      mockEvents.stateProjectAvailability.emit(payload)

      expect(listener).toHaveBeenCalledTimes(1)
      expect(listener).toHaveBeenCalledWith(payload)
      expect(mockEventBus.listenerCount()).toBe(1)
    } finally {
      stop()
    }

    mockEvents.stateProjectAvailability.emit(payload)

    expect(listener).toHaveBeenCalledTimes(1)
    expect(mockEventBus.listenerCount()).toBe(0)
  })

  it('scanProgress carries the generated payload type over the bus', () => {
    // T-076's §7.10 event on the mock side: the accessor is bound to
    // the wire name `scan://progress` with the generated payload
    // shape — the same subscribe/emit/unsubscribe lifetime pin as
    // availability's.
    const listener = vi.fn()
    const payload: ScanProgressEventWire = {
      checked: 14,
      found: 5,
      current_path: '/home/dev/src/repo-new',
    }
    const stop = mockEvents.scanProgress.on(listener)
    try {
      mockEvents.scanProgress.emit(payload)

      expect(listener).toHaveBeenCalledTimes(1)
      expect(listener).toHaveBeenCalledWith(payload)
      expect(mockEventBus.listenerCount()).toBe(1)
    } finally {
      stop()
    }

    mockEvents.scanProgress.emit(payload)

    expect(listener).toHaveBeenCalledTimes(1)
    expect(mockEventBus.listenerCount()).toBe(0)
  })

  it('syncProgress carries the generated payload type over the bus', () => {
    // T-119's §7.10 event on the mock side (D-202): the accessor is
    // bound to the wire name `sync://progress` with the generated
    // payload shape — the same subscribe/emit/unsubscribe lifetime
    // pin as availability's and scan's. The payload's vocabulary is
    // §9.4's own: the phase word, the decided count, the run total.
    const listener = vi.fn()
    const payload: SyncProgressEventWire = {
      phase: 'write',
      done: 4,
      total: 9,
    }
    const stop = mockEvents.syncProgress.on(listener)
    try {
      mockEvents.syncProgress.emit(payload)

      expect(listener).toHaveBeenCalledTimes(1)
      expect(listener).toHaveBeenCalledWith(payload)
      expect(mockEventBus.listenerCount()).toBe(1)
    } finally {
      stop()
    }

    mockEvents.syncProgress.emit(payload)

    expect(listener).toHaveBeenCalledTimes(1)
    expect(mockEventBus.listenerCount()).toBe(0)
  })

  it('binds exactly its own event: other names do not reach the accessor', () => {
    // The factory is the seam every future accessor comes from, so its
    // isolation is pinned once here, over a local bus.
    const local = createEventBus<TestEvents>()
    const progress = busEvent(local, 'scan://progress')
    const listener = vi.fn()
    progress.on(listener)

    local.emit('state://dirty', { projectIds: ['p1'] })

    expect(listener).not.toHaveBeenCalled()
  })
})

/** A small pinned world: 4 projects (last one archived), all available. */
const PROFILE: FixtureProfile = {
  seed: 21,
  projects: 4,
  instructions: 3,
  headings: 1,
  assignments: 5,
  archivedProjects: 1,
  variantsPerInstruction: 1,
  exclusions: 2,
}

/**
 * Subscribes to the mock's project-availability event for the duration of
 * one test body and ALWAYS unsubscribes — a failing assertion must not
 * leak a listener into the next test (the bus is module-global, and
 * `resetMockState` does not touch it). Awaits async bodies before
 * unsubscribing.
 */
async function withAvailabilityListener(
  body: (listener: ReturnType<typeof vi.fn>) => void | Promise<void>,
): Promise<void> {
  const listener = vi.fn()
  const stop = mockEvents.stateProjectAvailability.on(listener)
  try {
    await body(listener)
  } finally {
    stop()
  }
}

describe('applyMockAvailability', () => {
  beforeEach(() => {
    resetMockState(PROFILE)
  })

  it('emits one event with exactly the contradicting verdict, recorded for projects_list', async () => {
    await withAvailabilityListener(async (listener) => {
      const row = mockState().projects.find((r) => r.state === 'active')!

      const emitted = applyMockAvailability([
        { projectId: row.id, available: false, unavailableReason: 'missing' },
      ])

      const expected: ProjectAvailabilityEventWire = {
        changes: [{ project_id: row.id, available: false, unavailable_reason: 'missing' }],
      }
      expect(listener).toHaveBeenCalledTimes(1)
      expect(listener).toHaveBeenCalledWith(expected)
      expect(emitted).toEqual(expected)
      // Read-back through the command the UI actually calls, not the row
      // it maps from: `projects_list` must serve the new verdict.
      const column = (await mockCommands.projectsList(null)).find((p) => p.id === row.id)
      expect(column?.available).toBe(false)
      expect(column?.unavailable_reason).toBe('missing')
    })
  })

  it('is silent when the recorded verdict already holds', async () => {
    await withAvailabilityListener(async (listener) => {
      const row = mockState().projects.find((r) => r.state === 'active')!

      const first = applyMockAvailability([
        { projectId: row.id, available: false, unavailableReason: 'missing' },
      ])
      const repeated = applyMockAvailability([
        { projectId: row.id, available: false, unavailableReason: 'missing' },
      ])

      expect(first).not.toBeNull()
      expect(repeated).toBeNull()
      expect(listener).toHaveBeenCalledTimes(1)
    })
  })

  it('treats a never-probed gone folder as a change (the D-112 baseline)', async () => {
    // A 12-project world: index 10 is the D-09 fixture case (unavailable,
    // active), index 11 archived. The fixture row already serves the
    // verdict, but the session cache has not probed it — so the first
    // CONFIRMING verdict is a change, exactly like the backend's startup
    // probe of a dead folder (D-112: "a project first probed while its
    // folder is gone IS a change").
    resetMockState({ ...PROFILE, projects: 12 })
    await withAvailabilityListener(async (listener) => {
      const gone = mockState().projects.find((r) => r.state === 'active' && !r.available)!
      // The fixture's D-09 case at index 10: reason[10 % 4] = 'not_readable'.
      expect(gone.unavailableReason).toBe('not_readable')
      const reason: MockUnavailableReason = 'not_readable'

      const first = applyMockAvailability([
        { projectId: gone.id, available: false, unavailableReason: reason },
      ])
      const confirming = applyMockAvailability([
        { projectId: gone.id, available: false, unavailableReason: reason },
      ])

      expect(first).toEqual({
        changes: [{ project_id: gone.id, available: false, unavailable_reason: reason }],
      })
      expect(confirming).toBeNull()
      expect(listener).toHaveBeenCalledTimes(1)
    })
  })

  it('clears the reason when a project becomes available again', async () => {
    await withAvailabilityListener(async (listener) => {
      const row = mockState().projects.find((r) => r.state === 'active')!
      applyMockAvailability([{ projectId: row.id, available: false, unavailableReason: 'missing' }])

      const emitted = applyMockAvailability([{ projectId: row.id, available: true }])

      expect(emitted).toEqual({
        changes: [{ project_id: row.id, available: true, unavailable_reason: null }],
      })
      expect(listener).toHaveBeenCalledTimes(2)
      expect(row.available).toBe(true)
      expect(row.unavailableReason).toBeNull()
    })
  })

  it('treats a reason change on a still-unavailable project as a change', async () => {
    await withAvailabilityListener(async (listener) => {
      const row = mockState().projects.find((r) => r.state === 'active')!
      applyMockAvailability([{ projectId: row.id, available: false, unavailableReason: 'missing' }])

      const emitted = applyMockAvailability([
        { projectId: row.id, available: false, unavailableReason: 'not_readable' },
      ])

      expect(emitted).toEqual({
        changes: [{ project_id: row.id, available: false, unavailable_reason: 'not_readable' }],
      })
      expect(listener).toHaveBeenCalledTimes(2)
      expect(row.unavailableReason).toBe('not_readable')
    })
  })

  it('batches one pass into one event ordered by sort_order', async () => {
    await withAvailabilityListener(async (listener) => {
      const rows = mockState()
        .projects.filter((r) => r.state === 'active')
        .sort((a, b) => a.sortOrder - b.sortOrder)
      expect(rows.length).toBeGreaterThanOrEqual(2)
      const [first, second] = rows

      // Verdicts handed over in reverse order: the event must still come
      // out in probe order (sort_order).
      const emitted = applyMockAvailability([
        { projectId: second.id, available: false, unavailableReason: 'no_git_entry' },
        { projectId: first.id, available: false, unavailableReason: 'missing' },
      ])

      expect(emitted).toEqual({
        changes: [
          { project_id: first.id, available: false, unavailable_reason: 'missing' },
          { project_id: second.id, available: false, unavailable_reason: 'no_git_entry' },
        ],
      })
      expect(listener).toHaveBeenCalledTimes(1)
    })
  })

  it('keeps one change entry for a project listed twice, its last verdict', async () => {
    await withAvailabilityListener(async (listener) => {
      const row = mockState().projects.find((r) => r.state === 'active')!

      const emitted = applyMockAvailability([
        { projectId: row.id, available: false, unavailableReason: 'missing' },
        { projectId: row.id, available: false, unavailableReason: 'not_a_directory' },
      ])

      expect(emitted).toEqual({
        changes: [{ project_id: row.id, available: false, unavailable_reason: 'not_a_directory' }],
      })
      expect(listener).toHaveBeenCalledTimes(1)
    })
  })

  it('emits nothing for a pair that reverts within the pass (net effect)', async () => {
    await withAvailabilityListener(async (listener) => {
      const row = mockState().projects.find((r) => r.state === 'active')!

      const emitted = applyMockAvailability([
        { projectId: row.id, available: false, unavailableReason: 'missing' },
        { projectId: row.id, available: true },
      ])

      expect(emitted).toBeNull()
      expect(listener).not.toHaveBeenCalled()
      // The row still ends at the pass's net verdict.
      expect(row.available).toBe(true)
      expect(row.unavailableReason).toBeNull()
    })
  })

  it('an empty pass is silent (the event is never emitted with an empty diff)', async () => {
    await withAvailabilityListener(async (listener) => {
      expect(applyMockAvailability([])).toBeNull()
      expect(listener).not.toHaveBeenCalled()
    })
  })

  it('skips archived projects: the pass reads the active list', async () => {
    await withAvailabilityListener(async (listener) => {
      const archived = mockState().projects.find((r) => r.state === 'archived')!
      expect(archived).toBeDefined()

      const emitted = applyMockAvailability([
        { projectId: archived.id, available: false, unavailableReason: 'missing' },
      ])

      expect(emitted).toBeNull()
      expect(listener).not.toHaveBeenCalled()
      // Skipped, not recorded: a later re-check of the archived row has no
      // stale verdict to contradict.
      expect(archived.available).toBe(true)
    })
  })

  it('throws on an unknown project id instead of dropping it silently', async () => {
    await withAvailabilityListener(async (listener) => {
      const row = mockState().projects.find((r) => r.state === 'active')!

      // The whole pass is rejected before any side effect: the known row
      // must not be half-advanced with no event emitted.
      expect(() =>
        applyMockAvailability([
          { projectId: row.id, available: false, unavailableReason: 'missing' },
          { projectId: 'no-such-id', available: true },
        ]),
      ).toThrow(/unknown project id no-such-id/)

      expect(listener).not.toHaveBeenCalled()
      expect(row.available).toBe(true)
      expect(row.unavailableReason).toBeNull()
      // The cache half of all-or-nothing: a valid pass with the same
      // verdict must still EMIT — a pass that had polluted the session
      // cache before throwing would make it silent.
      expect(
        applyMockAvailability([
          { projectId: row.id, available: false, unavailableReason: 'missing' },
        ]),
      ).not.toBeNull()
      expect(listener).toHaveBeenCalledTimes(1)
    })
  })
})
