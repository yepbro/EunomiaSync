<script setup lang="ts">
// The base modal (T-363, UI-002): a native <dialog> carrying the
// mock-up's modal markup and classes (src/styles/modal.css). Open and
// close run through `v-model:open`: `true` calls showModal(), `false`
// calls close() — each guarded, because both throw on an already
// (not-)open dialog. The sync runs on mount as well (a parent can
// mount the dialog already open — the gallery does) and on every model
// change. The dialog's own `close` event — fired by the Esc key or by
// close() — writes the model back to false, so a parent's v-model
// stays truthful no matter who closed the dialog.
//
// What the native element gives for free (and is therefore NOT
// reimplemented here — focus trapping, full ARIA and the tests are
// card T-149's): Esc closes the dialog; on open the browser moves
// focus inside (to the first focusable element) and on close restores
// it to the previously focused element; everything outside an open
// modal dialog is inert; ::backdrop paints the dimmed page behind.
// This component adds only the accessible NAME (NFR-019): either
// `labelledBy` — the id of a visible title rendered by the slot
// content (the mock-up's dialogs label them this way) — or `label`,
// rendered as a visually hidden <h2> for dialogs without their own.
import { onMounted, computed, useTemplateRef, watch } from 'vue'

const open = defineModel<boolean>('open', { default: false })

const props = withDefaults(
  defineProps<{
    /** The dialog width: the mock-up's `.modal`, `.modal-lg`, `.modal-xl`. */
    size?: 'md' | 'lg' | 'xl'
    /** Id of a title element inside the slot content (the ARIA name). */
    labelledBy?: string
    /** Accessible name for dialogs without their own title element. */
    label?: string
  }>(),
  { size: 'md', labelledBy: undefined, label: undefined },
)

const dialog = useTemplateRef<HTMLDialogElement>('dialog')

// Mount sync: a dialog whose model is already true when it mounts (the
// gallery mounts screens open — v-model:open with a true ref) gets no
// watcher change, so the open state is applied here too. The guards
// mirror the watcher: showModal() throws on an already-open dialog,
// close() throws on an already-closed one.
onMounted(() => {
  const el = dialog.value
  if (el === null) return
  if (open.value && !el.open) el.showModal()
})

watch(open, (value) => {
  const el = dialog.value
  if (el === null) return
  if (value && !el.open) el.showModal()
  else if (!value && el.open) el.close()
})

// The size class maps onto the mock-up's width rules; `md` is the bare
// `.modal` (520px), so it needs no extra class.
const sizeClass = computed(() => (props.size === 'md' ? undefined : `modal-${props.size}`))
</script>

<template>
  <dialog
    ref="dialog"
    class="modal"
    :class="sizeClass"
    :aria-labelledby="props.labelledBy"
    :aria-label="props.label"
    @close="open = false"
  >
    <h2 v-if="props.label !== undefined && props.labelledBy === undefined" class="visually-hidden">
      {{ props.label }}
    </h2>
    <slot />
  </dialog>
</template>
