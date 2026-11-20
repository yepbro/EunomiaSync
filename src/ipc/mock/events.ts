// The mock event bus (T-141) — the subscription seam for §7.10 events.
//
// What exists today: `src/bindings.ts` (checked at task time) generates
// commands ONLY — tauri-specta has not emitted an `events` object yet, so
// there is no generated event type to wrap and the facade correctly serves
// none. The spec's event set (§7.10: `scan://progress`, `sync://progress`,
// `state://dirty`, `state://project-availability`, `state://file-changed`,
// per D-009's name registry) arrives with the backend tasks that emit
// them; when tauri-specta regenerates with `events`, the facade gains a
// typed `ipc.events` wrapper and `IpcEventMap` below is keyed with the
// generated payload types — nothing else changes.
//
// Until then this module provides the infrastructure those wrappers and
// the UI tests need: a small typed publish/subscribe bus with listener
// lifetimes (subscribe → unsubscribe), payload-typed per event name.

/** Listener lifetime handle: call to stop receiving. */
export type Unsubscribe = () => void

/**
 * A typed event bus. `Events` maps event names to their payload types;
 * with no generated payloads yet, `mockEventBus` is keyed loosely
 * (`Record<string, unknown>`) — stricter keys come with the generated
 * `events` object, and tests instantiate their own strictly-typed buses.
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
 * The bus the mock backend will broadcast §7.10 events on. Loose-keyed
 * until the generated `events` types exist (see the module docs).
 */
export const mockEventBus: EventBus<Record<string, unknown>> = createEventBus()
