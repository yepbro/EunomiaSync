/*
 * The dev screens registry (T-363; the matrix family's screens since
 * T-364): the screens the frontend skeleton has ported, listed by the
 * DEV-only gallery (DevGallery.vue) and opened through `#screen=<id>`.
 * The dialog contract for dialog components: wrap `BaseModal` and
 * expose `defineModel<boolean>('open')`; the gallery mounts them with
 * `v-model:open`. The showcase screens under src/dev/showcases/ are
 * small wrappers around the matrix components; they import the seed —
 * allowed here because the whole registry is reachable ONLY through
 * DevGallery.vue, which App.vue mounts behind the statically-replaced
 * DEV guard, so the registry, the showcases and the seed all ride the
 * same dead branch in a production build (the dist/ grep is the
 * standing proof; T-364's DECISIONS.md record).
 *
 * The registry titles are launcher data (dev-facing script strings,
 * rendered through interpolation, not template literals).
 */
import type { Component } from 'vue'
import MatrixCellStates from './showcases/MatrixCellStates.vue'
import ColumnHeaderStates from './showcases/ColumnHeaderStates.vue'
import ColumnMenuShowcase from './showcases/ColumnMenuShowcase.vue'
import VariantPopoverShowcase from './showcases/VariantPopoverShowcase.vue'
import TooltipShowcase from './showcases/TooltipShowcase.vue'
import BulkAssignDialog from '../components/matrix/BulkAssignDialog.vue'
import BulkConfirmDialog from '../components/matrix/BulkConfirmDialog.vue'

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

/** The ported screens, in launch order. */
export const screens: readonly Screen[] = [
  { id: 'shell', title: 'Main window (shell)', component: null },
  {
    id: 'matrix-cells',
    title: 'Matrix cells — the eight §11.2 states',
    component: MatrixCellStates,
  },
  {
    id: 'matrix-headers',
    title: 'Column headers — the five statuses',
    component: ColumnHeaderStates,
  },
  { id: 'matrix-menu', title: 'Column menu', component: ColumnMenuShowcase },
  { id: 'matrix-popover', title: 'Variant popover', component: VariantPopoverShowcase },
  { id: 'matrix-tooltip', title: 'Instruction text tooltip', component: TooltipShowcase },
  { id: 'matrix-bulk', title: 'Bulk assign dialog', component: BulkAssignDialog },
  { id: 'matrix-bulk-confirm', title: 'Bulk confirm dialog', component: BulkConfirmDialog },
]
