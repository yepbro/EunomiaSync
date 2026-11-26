import { describe, expect, it } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import App from './App.vue'
import { i18n } from './i18n'

// The shell contract (T-363): the main window renders the toolbar, the
// matrix frame and the sync panel, and it imports NO IPC module — the
// skeleton phase renders on the seed (src/dev/seed) through a
// DEV-guarded dynamic import, which the Vitest environment (DEV)
// resolves. The IPC facade claims of the old placeholder test are
// gone with the placeholder; T-212 wires app_info/projects_list back
// into this window and restores those assertions.
const mountApp = async () => {
  // The seed arrives through the DEV-only dynamic import in App.vue's
  // onMounted. The barrel spans the whole src/dev/seed/ tree, so the
  // module runner resolves it over several event-loop turns — more than
  // one flushPromises macrotask. Pre-loading the same module here makes
  // the component's import() settle from cache within a single
  // flushPromises, deterministically; the assertions below stay
  // unchanged and still run against the rendered DOM.
  await import('./dev/seed')
  const wrapper = mount(App, { global: { plugins: [i18n] } })
  await flushPromises()
  return wrapper
}

// The .vue raw-source scan (the i18n.spec.ts pattern): App.vue must
// not import the ipc facade — the seed is the shell's only data
// source in the skeleton phase. The import-statement shape keeps the
// check off App.vue's comments, which talk ABOUT the IPC handover.
const APP_SOURCES: Record<string, string> = import.meta.glob('./App.vue', {
  query: '?raw',
  import: 'default',
  eager: true,
})

describe('App (the main-window shell, T-363)', () => {
  it('renders the toolbar, the matrix frame and the sync panel', async () => {
    const wrapper = await mountApp()
    expect(wrapper.find('header.toolbar').exists()).toBe(true)
    expect(wrapper.find('main.matrix-wrap').exists()).toBe(true)
    expect(wrapper.find('footer.syncbar').exists()).toBe(true)
  })

  it('renders the localized level-1 window heading', async () => {
    const wrapper = await mountApp()
    const heading = wrapper.find('h1')
    expect(heading.exists()).toBe(true)
    expect(heading.text()).toBe('EunomiaSync')
  })

  it('feeds the project filter from the seed (All projects + the tracked set)', async () => {
    const wrapper = await mountApp()
    const options = wrapper.findAll('select.filter option')
    // The seed's eight demo projects plus the "All projects" option.
    expect(options.length).toBe(9)
    expect(options[0]?.text()).toBe('All projects')
    expect(options.map((option) => option.text())).toContain('EunomiaSync')
    expect(options.map((option) => option.text())).toContain('utils/deja-vu')
  })

  it('renders the seed hidden-instruction count next to "Show hidden"', async () => {
    const wrapper = await mountApp()
    // Two concealed rows in the demo library (Docstring format, PR
    // review checklist).
    expect(wrapper.find('.toggle-count').text()).toBe('2')
  })

  it('renders the seed sync facts (last-sync stamp, unsynced counter)', async () => {
    const wrapper = await mountApp()
    expect(wrapper.find('.sync-last strong').text()).toBe('19 Nov 2026, 14:32 (UTC+3)')
    expect(wrapper.find('.sync-counter .badge').text()).toBe('4')
    expect(wrapper.find('.sync-counter').text()).toContain('4 projects with unsynced changes')
  })

  it('keeps the selection cluster hidden while no rows are selected', async () => {
    const wrapper = await mountApp()
    // No selection state in the skeleton (UI-010 is the matrix card's).
    expect(wrapper.find('.selected-chip').attributes('hidden')).toBeDefined()
    expect(wrapper.find('header.toolbar .btn-ghost[hidden]').exists()).toBe(true)
  })

  it('renders the seed library rows — no empty state once rows exist', async () => {
    const wrapper = await mountApp()
    // T-364 renders the seed's library in the frame's tbody (9 headings
    // + 24 instructions); the UI-012 empty state shows only when the
    // row list is empty.
    expect(wrapper.find('.empty-state').exists()).toBe(false)
    expect(wrapper.findAll('.matrix tbody tr')).toHaveLength(33)
    expect(wrapper.find('.matrix tbody .row-heading').text()).toContain('General')
  })

  it('imports no IPC module', () => {
    for (const [file, source] of Object.entries(APP_SOURCES)) {
      expect(source, `${file} must not import the ipc facade`).not.toMatch(
        /from\s+'[^']*\/ipc[^']*'/,
      )
    }
    expect(Object.keys(APP_SOURCES).length).toBeGreaterThan(0)
  })
})
