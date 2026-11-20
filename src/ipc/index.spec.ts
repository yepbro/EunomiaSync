import { describe, expect, it } from 'vitest'
import { backend, ipc } from './index'

// The wiring canary: under the Vitest environment (`test.env` in
// vite.config.ts) the facade must have selected the mock backend, and a
// command call must resolve without any Tauri runtime in the page. The
// pure selection logic is covered in mode.spec.ts; this pins the actual
// module wiring.
describe('ipc facade wiring', () => {
  it('serves the mock backend under the test environment', () => {
    expect(backend).toBe('mock')
    expect(import.meta.env.VITE_IPC_MOCK).toBe('1')
  })

  it('answers projects_list with fixture data, no Tauri runtime involved', async () => {
    const projects = await ipc.commands.projectsList(null)
    expect(Array.isArray(projects)).toBe(true)
    expect(projects.length).toBeGreaterThan(0)
    for (const project of projects) {
      expect(project.state).toBe('active')
      expect(project.label.length).toBeGreaterThan(0)
    }
  })
})
