// Test support for the shell's specs (App.spec.ts, assets/theme.spec.ts).
// Mounting App.vue in Vitest must await the DEV-guarded seed import the
// component kicks off in onMounted (the harness fix of D-218, shared
// here so every shell spec mounts the same way). The barrel spans the
// whole src/dev/seed/ tree, so the module runner resolves it over
// several event-loop turns — more than one flushPromises macrotask.
// Pre-loading the same module here makes the component's import()
// settle from cache within a single flushPromises, deterministically;
// the specs' assertions stay unchanged and still run against the
// rendered DOM. Without the await, a spec that unmounts without
// flushing lets Vitest tear the environment down while the barrel is
// still loading — two unhandled EnvironmentTeardownError rejections
// and exit 1 on a green suite (the theme.spec.ts race, fixed
// 2026-11-26). TEST-ONLY: nothing under src/components/** or
// App.vue itself may import this module.
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import App from './App.vue'
import { i18n } from './i18n'

/** The mounted shell, seed rendered, ready to assert on. */
export async function mountApp(): Promise<VueWrapper> {
  await import('./dev/seed')
  const wrapper = mount(App, { global: { plugins: [i18n] } })
  await flushPromises()
  return wrapper
}
