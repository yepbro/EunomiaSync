// The mock-mode switch (T-141): one environment flag selects the backend
// the whole app talks to. Pure logic only — no `import.meta` access here —
// so the truth table is unit-testable without environment gymnastics; the
// facade wires the real flag value in (and inlines the comparison there:
// see the build-time note in `./index`).

/**
 * The environment flag that replaces the Tauri backend with the mock layer
 * (spec §15.1: UI tests and e2e run against "a mocked IPC backend").
 *
 * - Browser/UI development without Rust: `VITE_IPC_MOCK=1 pnpm dev`
 * - Vitest: always on (set through `test.env` in `vite.config.ts`)
 * - Playwright: `pnpm e2e` — playwright.config.ts passes the flag to its
 *   Vite webServer; `VITE_IPC_MOCK=1 pnpm e2e` works too.
 * - Production build: unset — the real generated `commands` are used.
 */
export const IPC_MOCK_ENV = 'VITE_IPC_MOCK'

/** The raw flag value as it arrives from `import.meta.env` (a string). */
export type MockFlagValue = string | undefined

/**
 * The accepted truthy spellings: `1` (CI/shell convention) and `true`.
 * Anything else — unset, `0`, a typo — means the real backend: a flag whose
 * value cannot be agreed on must not half-enable mocks. NOTE: the facade
 * inlines this comparison (build-time tree-shaking); extend both together.
 */
export function mockEnabled(raw: MockFlagValue): boolean {
  return raw === '1' || raw === 'true'
}

/** Which backend the facade is serving, for diagnostics and tests. */
export type IpcBackendKind = 'mock' | 'tauri'
