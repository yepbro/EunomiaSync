/*
 * The dev screens registry (T-363): the screens the frontend skeleton
 * has ported, listed by the DEV-only gallery (DevGallery.vue) and
 * opened through `#screen=<id>`. The family cards T-364…T-367 append
 * their dialogs here — the contract for a dialog component: it wraps
 * `BaseModal` and exposes `defineModel<boolean>('open')`; the gallery
 * mounts it with `v-model:open`.
 *
 * The registry itself is data, not UI copy: the titles label the dev
 * launcher (script-side strings, rendered through interpolation, not
 * template literals). This module must stay seed-free — the gallery
 * imports it statically, and everything statically reachable from it
 * would ship in the production bundle; the gallery is DEV-only and the
 * build's dist/ grep is the proof (T-363's DECISIONS.md record).
 */
import type { Component } from 'vue'

export type Screen = {
  /** Stable id used in the `#screen=<id>` hash. */
  readonly id: string
  /** The launcher's label (dev-facing data). */
  readonly title: string
  /**
   * The screen's component. `null` is the main window itself: it is
   * not mounted by the gallery — it IS the running app — and selecting
   * it just clears the hash (closing any gallery-opened screen).
   */
  readonly component: Component | null
}

/** The ported screens, in launch order. Starts with the shell only. */
export const screens: readonly Screen[] = [
  { id: 'shell', title: 'Main window (shell)', component: null },
]
