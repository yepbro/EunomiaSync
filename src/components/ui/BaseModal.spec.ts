import { afterEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import type { VueWrapper } from '@vue/test-utils'
import { nextTick } from 'vue'
import BaseModal from './BaseModal.vue'

// The modal's close contract (review H1, UI-003): the dialog's `close`
// event is where the form's result is readable — a <form method="dialog">
// submit writes the submitting button's value into returnValue, and
// returnValue persists between openings until reset (both facts
// verified in Chromium only). happy-dom does not implement dialog form
// submission, so these tests play the UA: they copy the markup's own
// button value into returnValue and dispatch `close` — the pinned
// contract is BaseModal's handling of the event. Esc fires `cancel`
// (cancelable) before close: `beforeClose` decides; a form submit is
// NOT interceptable by beforeClose (the dialog closes natively on
// submit — how buttons close is T-149's).

const Form = {
  template: '<form method="dialog"><button class="save" value="save">Save</button></form>',
}

// Each test's modal, unmounted in afterEach — a failed assertion must
// not leak an open dialog into the next test's document.
const mounted: VueWrapper[] = []
const mountModal = (props: Record<string, unknown> = {}): VueWrapper => {
  // The model is wired by hand: `onUpdate:open` writes back into the
  // prop, exactly what a parent's v-model:open does. The holder breaks
  // the mount-order cycle (the handler fires only after mount).
  const holder: { wrapper: VueWrapper | null } = { wrapper: null }
  const wrapper = mount(BaseModal, {
    props: {
      open: true,
      'onUpdate:open': (value: boolean) => holder.wrapper?.setProps({ open: value }),
      ...props,
    },
    slots: { default: Form },
  })
  holder.wrapper = wrapper
  mounted.push(wrapper)
  return wrapper
}

afterEach(() => {
  for (const wrapper of mounted.splice(0)) wrapper.unmount()
})

/** Chromium's submit-close, replayed for happy-dom (see the header):
 * the UA writes the submitting button's value into returnValue, closes
 * the dialog (no cancel event on this path) and fires `close`. */
async function submitLikeChromium(wrapper: VueWrapper, selector: string): Promise<void> {
  const dialog = wrapper.find('dialog').element as HTMLDialogElement
  const button = wrapper.find(selector).element as HTMLButtonElement
  dialog.returnValue = button.value
  dialog.open = false
  dialog.dispatchEvent(new Event('close'))
  await nextTick()
}

describe('BaseModal close contract', () => {
  it('a method="dialog" submit of value="save" emits close with "save" and sets the model false', async () => {
    const wrapper = mountModal()
    await submitLikeChromium(wrapper, '.save')
    expect(wrapper.emitted('close')).toEqual([['save']])
    expect(wrapper.emitted('update:open')).toEqual([[false]])
  })

  it('the result resets between openings: save once, reopen, close plainly → ""', async () => {
    const wrapper = mountModal()
    await submitLikeChromium(wrapper, '.save')
    expect(wrapper.emitted('close')).toEqual([['save']])
    await wrapper.setProps({ open: true })
    const dialog = wrapper.find('dialog').element as HTMLDialogElement
    expect(dialog.open).toBe(true)
    expect(dialog.returnValue).toBe('')
    dialog.close()
    await nextTick()
    expect(wrapper.emitted('close')).toEqual([['save'], ['']])
  })

  it('beforeClose returning false prevents the Esc cancel and keeps the dialog open', async () => {
    const wrapper = mountModal({ beforeClose: () => false })
    const dialog = wrapper.find('dialog').element as HTMLDialogElement
    const cancel = new Event('cancel', { cancelable: true })
    dialog.dispatchEvent(cancel)
    await nextTick()
    expect(cancel.defaultPrevented).toBe(true)
    expect(dialog.open).toBe(true)
    expect(wrapper.emitted('update:open')).toBeUndefined()
  })

  it('beforeClose returning true does not prevent the cancel', async () => {
    const wrapper = mountModal({ beforeClose: () => true })
    const dialog = wrapper.find('dialog').element as HTMLDialogElement
    const cancel = new Event('cancel', { cancelable: true })
    dialog.dispatchEvent(cancel)
    await nextTick()
    expect(cancel.defaultPrevented).toBe(false)
  })

  it('a promise verdict resolving false: prevented while pending, stays open', async () => {
    let resolve!: (allowed: boolean) => void
    const verdict = new Promise<boolean>((r) => {
      resolve = r
    })
    const wrapper = mountModal({ beforeClose: () => verdict })
    const dialog = wrapper.find('dialog').element as HTMLDialogElement
    const cancel = new Event('cancel', { cancelable: true })
    dialog.dispatchEvent(cancel)
    // The close is held synchronously (an async preventDefault would be
    // too late): prevented while the promise is pending, open after it
    // resolves false, and no close ever fires.
    expect(cancel.defaultPrevented).toBe(true)
    expect(dialog.open).toBe(true)
    resolve(false)
    await verdict
    await nextTick()
    expect(dialog.open).toBe(true)
    expect(wrapper.emitted('close')).toBeUndefined()
  })

  it('a promise verdict resolving true: prevented while pending, then closed', async () => {
    let resolve!: (allowed: boolean) => void
    const verdict = new Promise<boolean>((r) => {
      resolve = r
    })
    const wrapper = mountModal({ beforeClose: () => verdict })
    const dialog = wrapper.find('dialog').element as HTMLDialogElement
    const cancel = new Event('cancel', { cancelable: true })
    dialog.dispatchEvent(cancel)
    expect(cancel.defaultPrevented).toBe(true)
    expect(dialog.open).toBe(true)
    resolve(true)
    await verdict
    await nextTick()
    // The component closes the dialog itself once the verdict allows it.
    expect(dialog.open).toBe(false)
    expect(wrapper.emitted('close')).toEqual([['']])
  })
})
