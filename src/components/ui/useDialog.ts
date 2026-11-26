// The dialog-open state helper (T-363): a thin, documented wrapper
// around a boolean ref for `v-model:open` on `BaseModal`. Every future
// dialog component (the T-364…T-367 contract: wrap `BaseModal`, expose
// `defineModel<boolean>('open')`) keeps its open state through this —
// one line to change if the state ever needs more than a boolean
// (e.g. an opening/closing transition flag), zero call sites touched.
import { ref, type Ref } from 'vue'

/**
 * A `ref(false)` to bind as `v-model:open`:
 * `const open = useDialog()` → `<BaseModal v-model:open="open">`.
 */
export function useDialog(): Ref<boolean> {
  return ref(false)
}
