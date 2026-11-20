import { describe, expect, it, vi } from 'vitest'
import { createEventBus, mockEventBus } from './events'

/** A test-local payload map — the typing the generated events will bring. */
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
  it('is a working bus instance for the mock backend', () => {
    const listener = vi.fn()
    const unsubscribe = mockEventBus.on('sync://progress', listener)

    mockEventBus.emit('sync://progress', { phase: 'backups' })

    expect(listener).toHaveBeenCalledTimes(1)
    expect(listener).toHaveBeenCalledWith({ phase: 'backups' })
    unsubscribe()
    expect(mockEventBus.listenerCount()).toBe(0)
  })
})
