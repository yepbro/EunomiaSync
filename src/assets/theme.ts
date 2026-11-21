// The OS-theme bridge (T-145, spec UI-021): keeps the <html data-theme>
// attribute in step with the OS light/dark setting, so the CSS tokens in
// ./theme.css — and every `dark:` Tailwind utility, re-keyed to that
// attribute in main.css — follow the OS WITHOUT an app restart: the
// MediaQueryList `change` event fires while the app runs and the handler
// below just flips the attribute, no reload involved.
//
// Why an attribute and not a bare prefers-color-scheme token swap in CSS:
// the attribute is the single switch point the whole app shares (CSS
// variables, Tailwind's dark: variant, and any future manual theme
// override in settings), and it is observable from JS — the spec stubs
// matchMedia and flips `matches` to pin the live switch. Here the
// attribute is always DERIVED from the OS setting; nothing persists a
// user choice yet.
//
// The file lives under src/assets because T-145's parallel-window file
// scope allows src/assets/**; moving it next to other composables later
// is a no-op for callers (single import site today: App.vue).
import { getCurrentInstance, onUnmounted } from 'vue'

const DARK_QUERY = '(prefers-color-scheme: dark)'

/** The one write the whole theming system has: <html data-theme="…">. */
function applyTheme(root: HTMLElement, dark: boolean): void {
  root.dataset.theme = dark ? 'dark' : 'light'
}

/**
 * Bridges the OS color-scheme preference to the `data-theme` attribute on
 * `<html>`: applies it immediately (setup time — no light flash on a dark
 * OS) and re-applies on every OS `change` event. Returns a teardown
 * function; when called inside a component setup (App.vue), teardown is
 * also registered on unmount, so dev remounts stay clean.
 */
export function useOsTheme(): () => void {
  // No matchMedia (a bare DOM without the API): theming stays on the
  // light tokens and the app keeps rendering — the theme is decoration,
  // not a dependency.
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
    return () => {}
  }
  const mql = window.matchMedia(DARK_QUERY)
  const root = document.documentElement
  const onChange = (event: MediaQueryListEvent): void => applyTheme(root, event.matches)

  applyTheme(root, mql.matches)
  mql.addEventListener('change', onChange)

  const teardown = (): void => {
    mql.removeEventListener('change', onChange)
    delete root.dataset.theme
  }
  if (getCurrentInstance()) onUnmounted(teardown)
  return teardown
}
