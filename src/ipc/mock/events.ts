// The mock event bus (T-141) — the subscription seam for §7.10 events.
//
// `src/bindings.ts` generates all five of §7.10's events:
// `events.stateProjectAvailability` (T-079/D-112),
// `events.scanProgress` (T-076/D-153), `events.stateDirty` (T-101),
// `events.stateFileChanged` (T-102) and `events.syncProgress`
// (T-119/D-202). The bus below is keyed by `IpcEventMap` — the
// spec's event names mapped to their generated payload types
// (re-exported through `../contract`, D-003) — so mock emits and
// subscriptions are payload-typed against the real wire, and
// `mockEvents` exposes one typed accessor per event, named after the
// camelCase accessor the generated `events` object uses (the facade
// task will serve the real and mock sides behind one `ipc.events`).
// The map carries the events the mock's sources emit or its tests
// drive (`state://project-availability`, `scan://progress`,
// `sync://progress`); `state://dirty` and `state://file-changed`
// join it when their mock sources land (their payload types and
// Rust registrations already exist).
//
// The bus itself stays deliberately plain: a small typed
// publish/subscribe bus with listener lifetimes (subscribe →
// unsubscribe), payload-typed per event name, no Vue reactivity —
// components wrap it in their own subscriptions. The mock's event
// SOURCE for project availability lives in `./state`
// (`applyMockAvailability`, the D-112 diff semantics); the mock
// `syncRun` emits no progress the same way the mock `scanRun`
// stand-in emits none — the accessors are the mock contract, the
// emitting backend is the Rust pipeline's (the scan twin's posture,
// D-202's honest boundary). Tests for event names without mock
// sources instantiate their own strictly-typed buses with local
// payload maps.

import type {
  ProjectAvailabilityEventWire,
  ScanProgressEventWire,
  SyncProgressEventWire,
} from '../contract'

/** Listener lifetime handle: call to stop receiving. */
export type Unsubscribe = () => void

/**
 * A typed event bus. `Events` maps event names to their payload types;
 * `mockEventBus` is keyed by [`IpcEventMap`] (the generated payload
 * types), and tests instantiate their own strictly-typed buses.
 */
export type EventBus<Events extends Record<string, unknown>> = {
  /** Subscribes to one event; returns its unsubscribe handle. */
  on<K extends string & keyof Events>(name: K, listener: (payload: Events[K]) => void): Unsubscribe
  /** Emits one event to every current listener, in subscription order. */
  emit<K extends string & keyof Events>(name: K, payload: Events[K]): void
  /** How many listeners are attached (test/diagnostics aid). */
  listenerCount(): number
}

/** Creates a fresh, independent event bus. */
export function createEventBus<Events extends Record<string, unknown>>(): EventBus<Events> {
  const listeners = new Map<string, Set<(payload: unknown) => void>>()
  return {
    on(name, listener) {
      const set = listeners.get(name) ?? new Set()
      const first = set.size === 0
      if (first) listeners.set(name, set)
      set.add(listener as (payload: unknown) => void)
      return () => {
        set.delete(listener as (payload: unknown) => void)
        if (set.size === 0) listeners.delete(name)
      }
    },
    emit(name, payload) {
      const set = listeners.get(name)
      if (!set) return
      // Copy: a listener that unsubscribes mid-delivery must not change
      // what the current emission reaches.
      for (const listener of [...set]) listener(payload)
    },
    listenerCount() {
      let total = 0
      for (const set of listeners.values()) total += set.size
      return total
    },
  }
}

/**
 * The §7.10 events the mock backend broadcasts, keyed by the spec's wire
 * names and valued with the payload types generated into
 * `src/bindings.ts` (re-exported through `../contract` as the
 * `*_Serialize` wire halves — the shapes the Rust side emits).
 * `state://project-availability` (T-079/D-112), `scan://progress`
 * (T-076/D-153) and `sync://progress` (T-119/D-202). The next members
 * join with their mock sources, not with their Rust registration —
 * `state://dirty` and `state://file-changed` are already generated
 * and registered (`collect_events!` in `lib.rs`).
 */
export type IpcEventMap = {
  'state://project-availability': ProjectAvailabilityEventWire
  'scan://progress': ScanProgressEventWire
  'sync://progress': SyncProgressEventWire
}

/** One event of a bus bound to a standalone accessor (see [`busEvent`]). */
export type BusEvent<Payload> = {
  /** Subscribes to this event; returns its unsubscribe handle. */
  on(listener: (payload: Payload) => void): Unsubscribe
  /** Emits this event to every current listener, in subscription order. */
  emit(payload: Payload): void
}

/**
 * Binds one event of a bus into a standalone accessor, so frontend code
 * and tests subscribe by accessor name (`mockEvents.stateProjectAvailability`)
 * with the event's payload type — no wire-name strings scattered at call
 * sites. The factory behind the `mockEvents` object below.
 */
export function busEvent<
  Events extends Record<string, unknown>,
  Name extends string & keyof Events,
>(bus: EventBus<Events>, name: Name): BusEvent<Events[Name]> {
  return {
    on: (listener) => bus.on(name, listener),
    emit: (payload) => bus.emit(name, payload),
  }
}

/**
 * The bus the mock backend broadcasts §7.10 events on, keyed by
 * [`IpcEventMap`].
 */
export const mockEventBus: EventBus<IpcEventMap> = createEventBus()

/**
 * The mock backend's typed event accessors — one per generated event,
 * named after its camelCase accessor in `src/bindings.ts`
 * (`events.stateProjectAvailability`). On the real side only the
 * subscription halves of the generated accessors are usable from the
 * webview — `listen`/`once` behind the `core:event:allow-listen`/
 * `-unlisten` grant — while the generated `emit`/`emitTo` stay denied
 * (SEC-001: the Rust side emits); this accessor's synchronous `emit` is
 * a mock-only convenience for driving the bus from tests. Bridging the
 * real and mock sides behind one `ipc.events` is the facade task's
 * line, not the mock's.
 */
export const mockEvents = {
  stateProjectAvailability: busEvent(mockEventBus, 'state://project-availability'),
  scanProgress: busEvent(mockEventBus, 'scan://progress'),
  syncProgress: busEvent(mockEventBus, 'sync://progress'),
} as const
