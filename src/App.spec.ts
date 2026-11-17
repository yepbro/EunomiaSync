import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import App from './App.vue'

describe('App', () => {
  it('renders the application heading', () => {
    const wrapper = mount(App)
    const heading = wrapper.find('h1')
    expect(heading.exists()).toBe(true)
    expect(heading.text()).toBe('EunomiaSync')
  })
})
