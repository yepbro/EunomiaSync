/// <reference types="vite/client" />

declare module '*.vue' {
  import type { DefineComponent } from 'vue'
  const component: DefineComponent<{}, {}, any>
  export default component
}

// The IPC mock-layer switches (src/ipc/mode.ts, README → "Development
// without Rust"). Both are dev/test knobs: unset in production builds.
interface ImportMetaEnv {
  /** `1`/`true` swaps the IPC backend to the mock layer (src/ipc). */
  readonly VITE_IPC_MOCK?: string
  /** Which fixture profile the mock store builds: `s` (default), `m`, `l`. */
  readonly VITE_IPC_FIXTURES?: string
}
