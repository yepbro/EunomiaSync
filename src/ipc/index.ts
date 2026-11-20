// The IPC facade (T-141) — the app's single access point to Tauri.
//
// Components and stores import `ipc` from `src/ipc` and NEVER the
// generated bindings directly (D-003's line: no raw `invoke()`; this module
// is where a future ESLint pin scopes `../bindings` to `src/ipc/**`).
//
// The backend is chosen once, at module load, from the `VITE_IPC_MOCK`
// environment flag (see `./mode`):
//
// - `VITE_IPC_MOCK=1 pnpm dev`  — the UI runs in a plain browser, no Rust,
//   served by the mock layer over deterministic fixtures (README →
//   "Development without Rust").
// - Vitest sets the flag through `test.env` in `vite.config.ts`, so every
//   component test exercises the same mocks e2e will.
// - `pnpm e2e` — playwright.config.ts starts its Vite dev server with the
//   flag (webServer.env), the same switch (spec §15.1's "mocked IPC
//   backend").
// - Unset (production build, `tauri dev`) — the real generated commands.
//
// Events: `src/bindings.ts` generates none yet (spec §7.10 names five;
// they arrive with the emitting backend tasks), so the facade serves
// commands only today; the mock-side seam is `./mock/events`.

import { realCommands, type IpcCommands } from './contract'
import { mockCommands } from './mock/commands'
import type { IpcBackendKind } from './mode'

// The selection is written INLINE (not through `mockEnabled()`) on purpose:
// Vite replaces `import.meta.env.VITE_IPC_MOCK` with a literal at build
// time, the comparison folds, and the bundler drops the entire mock layer
// from the production bundle (verified: no mock strings in `dist/`). A
// function call here would keep the branch opaque and ship ~5 kB gzip of
// dead mock code. The accepted spellings must stay in sync with
// `mockEnabled` in ./mode — its spec (mode.spec.ts) is the truth table.
const MOCK_FLAG = import.meta.env.VITE_IPC_MOCK
const mocked = MOCK_FLAG === '1' || MOCK_FLAG === 'true'

/** The backend this page load serves ('mock' or 'tauri'). */
export const backend: IpcBackendKind = mocked ? 'mock' : 'tauri'

/** The app's commands: the generated surface, mock-backed when flagged. */
export const commands: IpcCommands = mocked ? mockCommands : realCommands

/** The facade object — import this, not the internals above. */
export const ipc = { backend, commands } as const

export { IPC_MOCK_ENV } from './mode'
export type { IpcBackendKind }
