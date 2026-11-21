import { describe, expect, it } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import App from './App.vue'
import { i18n } from './i18n'
import type { FixtureProfile } from './ipc/fixtures/generator'
import { resetMockState } from './ipc/mock/state'

// Integration proof for the mock IPC layer (T-141) and the i18n catalog
// (T-142): the component tree talks to `app_info` and `projects_list`
// through the src/ipc facade, and under the Vitest environment that
// facade is mock-backed (vite.config.ts `test.env`) — the same mocks
// browser dev and Playwright use. Every user-visible string renders from
// a vue-i18n key, including the pluralized tracked-projects line.
const mountApp = () => mount(App, { global: { plugins: [i18n] } })

/** A minimal deterministic world with exactly `projects` active projects. */
const profileWith = (projects: number): FixtureProfile => ({
  seed: 7,
  projects,
  instructions: 0,
  headings: 0,
  assignments: 0,
  archivedProjects: 0,
  variantsPerInstruction: 1,
  exclusions: 0,
})

describe('App', () => {
  it('renders the localized application heading', () => {
    const wrapper = mountApp()
    const heading = wrapper.find('h1')
    expect(heading.exists()).toBe(true)
    expect(heading.text()).toBe('EunomiaSync')
  })

  it('renders the localized tagline and logo alts', () => {
    // Pins the keys the version/plural tests do not touch — a typo in
    // `app.tagline` or `app.logoAlt.*` renders the literal key string
    // (vue-i18n only warns), so these assertions are its only guard.
    const wrapper = mountApp()
    expect(wrapper.find('main > p:last-of-type').text()).toBe(
      'Click on the Tauri, Vite, and Vue logos to learn more.',
    )
    const alts = wrapper.findAll('img').map((img) => img.attributes('alt'))
    expect(alts).toEqual(['Vite logo', 'Tauri logo', 'Vue logo'])
  })

  it('shows the version served by the mock IPC backend', async () => {
    const wrapper = mountApp()
    await flushPromises()
    const version = wrapper.find('.version')
    expect(version.exists()).toBe(true)
    // The interpolated `app.versionLine` key ("v{version}").
    expect(version.text()).toBe('v0.1.0')
  })

  // The UI-020 plural criterion, rendered end to end: `projects_list`
  // answers through the mock facade, and the count line picks the
  // one/other plural form of `app.trackedProjects`.
  it.each([
    [1, '1 project tracked'],
    [2, '2 projects tracked'],
    [5, '5 projects tracked'],
  ])('pluralizes the tracked-projects line (%i project(s) → "%s")', async (count, expected) => {
    resetMockState(profileWith(count))
    const wrapper = mountApp()
    await flushPromises()
    expect(wrapper.find('.projects').text()).toBe(expected)
  })

  it('counts only active projects (archived rows stay out of the line)', async () => {
    // 3 projects, 2 of them archived: `projectsList(false)` must answer
    // with 1 active column — this is what separates counting the active
    // set from counting the whole table.
    resetMockState({ ...profileWith(3), archivedProjects: 2 })
    const wrapper = mountApp()
    await flushPromises()
    expect(wrapper.find('.projects').text()).toBe('1 project tracked')
  })

  it('renders the zero form of the tracked-projects line for an empty world', async () => {
    resetMockState(profileWith(0))
    const wrapper = mountApp()
    await flushPromises()
    expect(wrapper.find('.projects').text()).toBe('no projects tracked yet')
  })
})
