import { describe, expect, it } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import App from './App.vue'

// Integration proof for the mock IPC layer (T-141): the component tree
// talks to `app_info` through the src/ipc facade, and under the Vitest
// environment that facade is mock-backed (vite.config.ts `test.env`) —
// the same mocks browser dev and Playwright use.
describe('App', () => {
  it('renders the application heading', () => {
    const wrapper = mount(App)
    const heading = wrapper.find('h1')
    expect(heading.exists()).toBe(true)
    expect(heading.text()).toBe('EunomiaSync')
  })

  it('shows the version served by the mock IPC backend', async () => {
    const wrapper = mount(App)
    await flushPromises()
    const version = wrapper.find('.version')
    expect(version.exists()).toBe(true)
    expect(version.text()).toBe('v0.1.0')
  })
})
