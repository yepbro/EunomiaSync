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
// The close result (review H1): a <form method="dialog"> submit writes
// the submitting button's value into `returnValue`, and the dialog's
// `close` event is the only place it is readable. The component reads
// it there and emits a typed `close` with the result ('' when Esc or a
// plain close() ended the dialog). Chromium facts — verified in
// Chromium only: returnValue persists between openings, so it is
// reset to '' before every showModal(). LIMIT: a form with
// method="dialog" closes natively ON SUBMIT, and that path fires no
// `cancel` event — `beforeClose` guards the ESC path only; how the
// action buttons close is T-149's contract work.
//
// The Esc guard (review H1; UI-003's unsaved-changes need): Esc fires
// `cancel` (cancelable) before the close. `beforeClose` is consulted
// with the event pending: a `false` verdict preventDefaults it and the
// dialog stays open; `true` closes as usual. A PROMISE verdict cannot
// influence the already-dispatched event (an async preventDefault is
// too late), so the close is held synchronously and the resolved
// verdict decides whether the dialog then closes itself.
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
    /**
     * The Esc guard (UI-003): return (or resolve to) false from the
     * dialog's `cancel` event and the dialog stays open. Not consulted
     * for a method="dialog" submit — that closes natively (T-149 owns
     * how the buttons close).
     */
    beforeClose?: () => boolean | Promise<boolean>
  }>(),
  { size: 'md', labelledBy: undefined, label: undefined, beforeClose: undefined },
)

const emit = defineEmits<{ close: [result: string] }>()

const dialog = useTemplateRef<HTMLDialogElement>('dialog')

// Mount sync: a dialog whose model is already true when it mounts (the
// gallery mounts screens open — v-model:open with a true ref) gets no
// watcher change, so the open state is applied here too. The guards
// mirror the watcher: showModal() throws on an already-open dialog; the
// `!el.open` guard keeps the close side symmetric (close() itself is a
// no-op on an already-closed dialog). The reset before every
// showModal(): returnValue persists between openings in Chromium
// (review H1) and a stale result must not survive a new opening.
const showModal = (el: HTMLDialogElement): void => {
  el.returnValue = ''
  el.showModal()
}

onMounted(() => {
  const el = dialog.value
  if (el === null) return
  if (open.value && !el.open) showModal(el)
})

watch(open, (value) => {
  const el = dialog.value
  if (el === null) return
  if (value && !el.open) showModal(el)
  else if (!value && el.open) el.close()
})

// The dialog's own `close` event carries the form's result: the
// submitting button's `value` (a string; '' for Esc or close()),
// re-emitted typed; the model goes false either way.
const onDialogClose = (event: Event): void => {
  const result = event.target instanceof HTMLDialogElement ? event.target.returnValue : ''
  open.value = false
  emit('close', result)
}

// Esc arrives as a cancelable `cancel` event before the close. A sync
// boolean verdict decides inline; a promise verdict holds the close
// synchronously (the dispatch has returned by the time it resolves)
// and closes the dialog itself when allowed.
const onDialogCancel = (event: Event): void => {
  const guard = props.beforeClose
  if (guard === undefined) return
  const verdict = guard()
  if (verdict instanceof Promise) {
    event.preventDefault()
    void verdict.then((allowed) => {
      const el = dialog.value
      if (allowed && el !== null && el.open) el.close()
    })
    return
  }
  if (!verdict) event.preventDefault()
}

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
    @close="onDialogClose"
    @cancel="onDialogCancel"
  >
    <h2 v-if="props.label !== undefined && props.labelledBy === undefined" class="visually-hidden">
      {{ props.label }}
    </h2>
    <slot />
  </dialog>
</template>
