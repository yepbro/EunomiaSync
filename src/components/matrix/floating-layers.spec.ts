import { afterEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import type { VueWrapper } from '@vue/test-utils'
import { defineComponent, nextTick, ref } from 'vue'
import MatrixCell from './MatrixCell.vue'
import ProjectColumnHeader from './ProjectColumnHeader.vue'
import { useDismiss } from './floating'
import { i18n } from '../../i18n'
import type { Assignment, LibraryInstruction, Project } from '../../dev/seed'

// The one-floating-layer-per-kind invariant (review H2; MAIN-032,
// MAIN-033): at most one variant popover and at most one column menu
// is open — opening a second closes the first, and an outside click
// closes the open one. The review reproduced the defect on the pre-fix
// code with two instances and real triggers (two popovers open,
// aria-expanded ["true","true"]); this spec is that repro as a
// regression test.

const project = (name: string, label: string): Project => ({
  name,
  label,
  color: '#2563eb',
  description: '',
  path: `/srv/${name}`,
  origin: 'manual',
  scheme: 'A',
  baseFile: 'AGENTS.md',
  extraLocal: false,
  extraLocalPos: 'end',
  extraProject: false,
  extraProjectPos: 'end',
  syncedAt: '19 Nov 2026, 09:14 (UTC+3)',
  runs: 3,
  status: 'ok',
})

const instruction: LibraryInstruction = {
  kind: 'instruction',
  name: 'Code style',
  depth: 1,
  text: 'Run the formatter.',
  hidden: false,
  variants: [],
}

const assignment: Assignment = { state: 'main', dirty: false }

// Each test's wrappers, unmounted in afterEach — a failed assertion
// must not leak live dismiss listeners into the next test.
const mounted: VueWrapper[] = []
const track = (wrapper: VueWrapper): VueWrapper => {
  mounted.push(wrapper)
  return wrapper
}

afterEach(() => {
  for (const wrapper of mounted.splice(0)) wrapper.unmount()
})

const mountCell = (project: Project): VueWrapper =>
  track(
    mount(MatrixCell, {
      props: { row: instruction, project, assignment },
      global: { plugins: [i18n] },
      // Attached: the eye's click must bubble to the real document for
      // the dismiss path to run — a detached wrapper would pass the
      // dual-open assertions on the exclusive-slot invariant alone.
      attachTo: document.body,
    }),
  )

const mountHeader = (project: Project): VueWrapper =>
  track(
    mount(ProjectColumnHeader, {
      props: { project },
      global: { plugins: [i18n] },
      attachTo: document.body,
    }),
  )

describe('one floating layer per kind (H2)', () => {
  it('opening cell B’s popover closes cell A’s (aria-expanded false/true)', async () => {
    const a = mountCell(project('proj-a', 'A'))
    const b = mountCell(project('proj-b', 'B'))
    await a.find('.cell-icon--eye').trigger('click')
    expect(a.find('.variants-popover').exists()).toBe(true)
    expect(b.find('.variants-popover').exists()).toBe(false)
    expect(a.find('.cell-icon--eye').attributes('aria-expanded')).toBe('true')
    await b.find('.cell-icon--eye').trigger('click')
    expect(a.find('.variants-popover').exists()).toBe(false)
    expect(b.find('.variants-popover').exists()).toBe(true)
    expect(a.find('.cell-icon--eye').attributes('aria-expanded')).toBe('false')
    expect(b.find('.cell-icon--eye').attributes('aria-expanded')).toBe('true')
  })

  it('opening header B’s column menu closes header A’s', async () => {
    const a = mountHeader(project('proj-a', 'A'))
    const b = mountHeader(project('proj-b', 'B'))
    await a.find('.col-menu-btn').trigger('click')
    expect(a.find('.column-menu').exists()).toBe(true)
    expect(b.find('.column-menu').exists()).toBe(false)
    expect(a.find('.col-menu-btn').attributes('aria-expanded')).toBe('true')
    await b.find('.col-menu-btn').trigger('click')
    expect(a.find('.column-menu').exists()).toBe(false)
    expect(b.find('.column-menu').exists()).toBe(true)
    expect(a.find('.col-menu-btn').attributes('aria-expanded')).toBe('false')
    expect(b.find('.col-menu-btn').attributes('aria-expanded')).toBe('true')
  })

  it('an outside click closes the open popover', async () => {
    const a = mountCell(project('proj-a', 'A'))
    await a.find('.cell-icon--eye').trigger('click')
    expect(a.find('.variants-popover').exists()).toBe(true)
    document.body.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()
    expect(a.find('.variants-popover').exists()).toBe(false)
    expect(a.find('.cell-icon--eye').attributes('aria-expanded')).toBe('false')
  })

  it('useDismiss ignores only its OWN trigger — a same-class foreign button closes it', async () => {
    const closed: string[] = []
    // A minimal floating layer wired through the same useDismiss
    // contract the popover and the menu use, attached so the clicks
    // reach the document listener. The own-trigger check must compare
    // ELEMENTS: with the old class-based ignore (.layer-trigger), the
    // foreign button below would be ignored too and the layer would
    // stay open — the H2 defect inside a single component.
    const Layer = defineComponent({
      setup() {
        const box = ref<HTMLElement | null>(null)
        const trigger = ref<Element | null>(null)
        useDismiss(box, () => closed.push('A'), trigger)
        return { box, trigger }
      },
      template:
        '<span><button class="layer-trigger" ref="trigger">own</button>' +
        '<button class="layer-trigger" data-foreign>foreign</button>' +
        '<div ref="box" class="layer-box">b</div></span>',
    })
    const a = track(mount(Layer, { attachTo: document.body }))
    // The layer's own registered trigger: ignored.
    await a.find('button.layer-trigger:not([data-foreign])').trigger('click')
    expect(closed).toEqual([])
    // The foreign button carries the SAME class but is not this
    // layer's trigger: the dismiss must fire.
    await a.find('[data-foreign]').trigger('click')
    expect(closed).toEqual(['A'])
  })
})
