// Theme behavior and contrast proofs (T-145, spec UI-021/NFR-019/NFR-022,
// §11.5/§12.4). Two acceptance criteria live here:
//
//  1. The OS theme switch reaches the app WITHOUT a restart: useOsTheme
//     (./theme.ts) subscribes to matchMedia('(prefers-color-scheme:
//     dark)') and flips <html data-theme> when the OS changes — the
//     tests below stub matchMedia and flip `matches` at runtime.
//  2. Text contrast >= 4.5:1 in BOTH themes: the palette (./theme.css)
//     is parsed here and every base text/background pair is checked with
//     the WCAG relative-luminance formula — no external dependency, the
//     checker itself is pinned against known WCAG reference pairs first.
//
// The CSS files are asserted as shipped: the stylesheet IS the artifact
// the webview renders, so a pair that fails here fails in the app. They
// are read from disk with node:fs — vitest's default css handling stubs
// CSS imports (plain, `?raw` and `?inline` all come back empty), and
// reading the source file needs no config change.
//
// @ts-expect-error node builtins are not part of the src tsconfig
// project (it declares no node types; adding them is a tsconfig change
// outside this task's file scope). At runtime vitest runs in Node, so
// the import resolves — once node types are added this directive
// becomes unused and typecheck FAILS until it is deleted (the cue to
// remove it).
import { readFileSync } from 'node:fs'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import App from '../App.vue'
import { i18n } from '../i18n'
import { useOsTheme } from './theme'

const readSource = (name: string): string => readFileSync(new URL(name, import.meta.url), 'utf8')

const themeCss = readSource('./theme.css')
const mainCss = readSource('./main.css')
const mainTs = readSource('../main.ts')

// ---------------------------------------------------------------------------
// WCAG 2.x contrast math (https://www.w3.org/TR/WCAG21/#dfn-relative-luminance)
// ---------------------------------------------------------------------------

/** Linearizes one 8-bit sRGB channel per the WCAG definition. */
function channel(c8: number): number {
  const c = c8 / 255
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
}

/** Relative luminance of a `#rrggbb` color. */
function luminance(hex: string): number {
  const digits = hex.slice(1)
  const [r, g, b] = [0, 2, 4].map((i) => channel(Number.parseInt(digits.slice(i, i + 2), 16)))
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

/** WCAG contrast ratio of two `#rrggbb` colors (1..21, order-independent). */
function contrast(fg: string, bg: string): number {
  const l1 = luminance(fg)
  const l2 = luminance(bg)
  return (Math.max(l1, l2) + 0.05) / (Math.min(l1, l2) + 0.05)
}

describe('WCAG contrast checker (pinned against reference pairs)', () => {
  it('black on white is exactly 21:1', () => {
    expect(contrast('#000000', '#ffffff')).toBe(21)
  })

  it('a color on itself is exactly 1:1', () => {
    expect(contrast('#123456', '#123456')).toBe(1)
  })

  it('detects the AA boundary: #767676 passes, #777777 fails on white', () => {
    // The classic WCAG reference pair: #767676 is the lightest gray that
    // still clears 4.5:1 on white (~4.54), #777777 does not (~4.48). A
    // checker that cannot reproduce both cannot be trusted for the
    // palette assertions below.
    expect(contrast('#767676', '#ffffff')).toBeGreaterThanOrEqual(4.5)
    expect(contrast('#777777', '#ffffff')).toBeLessThan(4.5)
  })
})

// ---------------------------------------------------------------------------
// theme.css parsing: comments out, blocks extracted, tokens collected
// ---------------------------------------------------------------------------

/** Removes CSS comments so prose examples cannot fake tokens. */
function stripComments(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, '')
}

/** Returns the declaration body of the first `{ … }` block after `selector`. */
function extractBlock(css: string, selector: string): string {
  const start = css.indexOf(selector)
  if (start === -1) throw new Error(`selector not found in stylesheet: ${selector}`)
  const open = css.indexOf('{', start)
  let depth = 0
  for (let i = open; i < css.length; i++) {
    if (css[i] === '{') depth++
    else if (css[i] === '}') {
      depth--
      if (depth === 0) return css.slice(open + 1, i)
    }
  }
  throw new Error(`unbalanced braces after selector: ${selector}`)
}

/** Collects `--token: #rrggbb` pairs from a declaration block. */
function parseTokens(block: string): Record<string, string> {
  const tokens: Record<string, string> = {}
  for (const match of block.matchAll(/--([\w-]+):\s*(#[0-9a-fA-F]{6})\b/g)) {
    tokens[match[1]] = match[2].toLowerCase()
  }
  return tokens
}

const bareThemeCss = stripComments(themeCss)
const lightTokens = parseTokens(extractBlock(bareThemeCss, ':root'))
const darkTokens = parseTokens(extractBlock(bareThemeCss, "[data-theme='dark']"))
// The media-query fallback block (the pre-JS / no-JS dark paint).
const fallbackTokens = parseTokens(extractBlock(bareThemeCss, ":root:not([data-theme='light'])"))

/** Reads a token or fails loudly (a missing token would otherwise skip its contrast check). */
function token(theme: Record<string, string>, name: string): string {
  const value = theme[name]
  if (value === undefined) throw new Error(`theme token not defined: ${name}`)
  return value
}

// ---------------------------------------------------------------------------
// Palette structure
// ---------------------------------------------------------------------------

describe('theme.css palette structure', () => {
  it('keeps the documented token inventory in the light block', () => {
    expect(Object.keys(lightTokens).sort()).toEqual([
      'accent',
      'accent-soft',
      'accent-strong',
      'danger',
      'focus-ring',
      'ink',
      'ink-muted',
      'line',
      'on-accent',
      'page',
      'surface',
      'surface-2',
      'warning',
    ])
  })

  it('overrides every token in the dark block (no silent light fallbacks)', () => {
    // A token missing from [data-theme='dark'] silently keeps its light
    // value in the dark theme — this pins the two sets identical.
    expect(Object.keys(darkTokens).sort()).toEqual(Object.keys(lightTokens).sort())
  })

  it('ships a CSS-only dark fallback for the first paint (no white flash, no JS needed)', () => {
    // Before the JS bundle runs (and if it never runs), the OS setting
    // itself must paint the dark theme: the media-query fallback block
    // has to exist and carry EXACTLY the attribute block's values — two
    // hand-maintained dark sets drift, so they are pinned equal.
    expect(bareThemeCss).toMatch(/@media \(prefers-color-scheme: dark\)/)
    expect(fallbackTokens).toEqual(darkTokens)
  })

  it('declares color-scheme per theme so native UI (scrollbars, form controls) follows', () => {
    expect(extractBlock(bareThemeCss, ':root')).toMatch(/color-scheme:\s*light/)
    expect(extractBlock(bareThemeCss, "[data-theme='dark']")).toMatch(/color-scheme:\s*dark/)
    // The fallback block too: it alone rules the dark-OS first paint,
    // where native scrollbars/controls must already be dark.
    expect(extractBlock(bareThemeCss, ":root:not([data-theme='light'])")).toMatch(
      /color-scheme:\s*dark/,
    )
  })
})

// ---------------------------------------------------------------------------
// Contrast: every base text pair in both themes >= 4.5:1 (UI-021, NFR-019)
// ---------------------------------------------------------------------------

/** Text pairs: foreground token on background token, required >= 4.5:1. */
const TEXT_PAIRS: Array<[fg: string, bg: string]> = [
  ['ink', 'page'],
  ['ink', 'surface'],
  ['ink', 'surface-2'],
  ['ink-muted', 'page'],
  ['ink-muted', 'surface'],
  ['ink-muted', 'surface-2'],
  ['accent', 'page'],
  ['accent', 'surface'],
  ['accent', 'surface-2'],
  // The primary button and the soft accent background carry their own
  // label/foreground pairs (on-accent, accent-strong).
  ['accent-strong', 'accent-soft'],
  ['on-accent', 'accent-strong'],
  ['warning', 'page'],
  ['warning', 'surface'],
  ['warning', 'surface-2'],
  ['danger', 'page'],
  ['danger', 'surface'],
  ['danger', 'surface-2'],
]

/** Non-text pairs (focus indicator, UI boundaries): required >= 3:1 (WCAG 1.4.11). */
const NON_TEXT_PAIRS: Array<[fg: string, bg: string]> = [
  ['focus-ring', 'page'],
  ['focus-ring', 'surface'],
  ['line', 'page'],
  ['line', 'surface'],
  // The primary button's fill against the panel it sits on: its visual
  // boundary must survive without a border.
  ['accent-strong', 'surface'],
]

describe.each([
  ['light', lightTokens],
  ['dark', darkTokens],
] as const)('%s theme contrast', (theme, tokens) => {
  it.each(TEXT_PAIRS)('%s on %s is at least 4.5:1', (fg, bg) => {
    const ratio = contrast(token(tokens, fg), token(tokens, bg))
    expect(ratio, `${theme}: ${fg} on ${bg} was ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(4.5)
  })

  it.each(NON_TEXT_PAIRS)('%s on %s is at least 3:1 (non-text)', (fg, bg) => {
    const ratio = contrast(token(tokens, fg), token(tokens, bg))
    expect(ratio, `${theme}: ${fg} on ${bg} was ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(3)
  })

  it('leaves no token without a contrast pair', () => {
    // The pair lists are hand-maintained (they encode each token's
    // role): without this guard a NEW token could ship with zero
    // contrast coverage while every listed pair still passes.
    const covered = new Set([...TEXT_PAIRS, ...NON_TEXT_PAIRS].flat())
    for (const name of Object.keys(tokens)) {
      if (name === 'page' || name === 'surface') continue // backgrounds, not foregrounds
      expect(
        covered,
        `token --${name} has no contrast pair in TEXT_PAIRS/NON_TEXT_PAIRS`,
      ).toContain(name)
    }
  })
})

// The AppErrorAlert (T-143, outside this task's edit scope) paints its own
// hardcoded surfaces while inheriting the shell's --ink text color. These
// literals mirror its scoped styles: the alert background and the envelope
// <pre> background per theme. If that component's palette ever changes,
// update the mirrors. NOTE: the component switches on the raw media query
// (its own styles), not on the data-theme attribute, so this cross-check
// holds while the attribute tracks the OS — exactly what useOsTheme()
// guarantees today. If the attribute ever diverges (a persisted manual
// override), the component must move to the attribute first (a follow-up
// on its owner's plate, recorded in D-117).
describe('AppErrorAlert surfaces keep AA contrast against the theme ink', () => {
  const alertSurfaces: Record<'light' | 'dark', { ink: string; surfaces: string[] }> = {
    light: { ink: token(lightTokens, 'ink'), surfaces: ['#fef2f2', '#f3f4f6'] },
    dark: { ink: token(darkTokens, 'ink'), surfaces: ['#2c1a1a', '#1f1f1f'] },
  }

  // One source of truth: the rows are driven from alertSurfaces, so the
  // mirror table above is the only place to update.
  const alertCases = (['light', 'dark'] as const).flatMap((theme) =>
    alertSurfaces[theme].surfaces.map((surface) => ({
      theme,
      ink: alertSurfaces[theme].ink,
      surface,
    })),
  )

  it.each(alertCases)(
    '$theme alert surface $surface holds 4.5:1 with the theme ink',
    ({ ink, surface }) => {
      const ratio = contrast(ink, surface)
      expect(ratio, `ink ${ink} on ${surface} was ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(
        4.5,
      )
    },
  )
})

// ---------------------------------------------------------------------------
// main.css: utilities mapping + the app-wide a11y norms (UI-021/NFR-022)
// ---------------------------------------------------------------------------

describe('main.css base layer', () => {
  it('loads the palette (main.css imports theme.css, main.ts imports main.css)', () => {
    // The palette's only route into the app: drop either import and every
    // utility resolves an undefined custom property while every other
    // test here stays green (they read the files directly).
    expect(stripComments(mainCss)).toMatch(/@import\s+'\.\/theme\.css'/)
    expect(mainTs).toMatch(/import\s+'\.\/assets\/main\.css'/)
  })

  it('maps every palette token to Tailwind utilities (@theme inline)', () => {
    // A token without its --color-* mapping silently has no bg-, text-
    // or border- class — screens would fall back to unstyled defaults.
    const css = stripComments(mainCss)
    for (const name of Object.keys(lightTokens)) {
      expect(css).toMatch(new RegExp(`--color-${name}:\\s*var\\(--${name}\\)`))
    }
  })

  it("keys Tailwind's dark: variant on the data-theme attribute (single switch point)", () => {
    // No dark: consumer exists yet (the shell uses only theme-agnostic
    // tokens); this pin keeps the variant on the attribute switch so the
    // first screen that uses dark: utilities joins the single-switch
    // design instead of silently reverting to the raw media query.
    expect(stripComments(mainCss)).toMatch(/@custom-variant dark[^;]*\[data-theme=['"]dark['"]\]/)
  })

  it('ships a global visible focus ring (UI-021)', () => {
    expect(stripComments(mainCss)).toMatch(
      /:focus-visible\s*\{[^}]*outline:\s*2px solid var\(--focus-ring\)[^}]*outline-offset:\s*2px/,
    )
  })

  it('enforces the 24px minimum for interactive controls (NFR-022)', () => {
    const css = stripComments(mainCss)
    const rule = css.match(/([^{}]+)\{[^}]*min-height:\s*1\.5rem[^}]*min-width:\s*1\.5rem/)
    if (rule === null) throw new Error('no min-height/min-width 1.5rem rule found in main.css')
    const selector = rule[1]
    for (const control of ['button', "[role='button']", 'summary', 'input', 'select', 'textarea']) {
      expect(selector, `tap-target rule must cover <${control}>`).toContain(control)
    }
  })
})

// ---------------------------------------------------------------------------
// useOsTheme: live OS theme switching without a restart
// ---------------------------------------------------------------------------

type ChangeListener = (event: { matches: boolean }) => void

/** A controllable matchMedia('(prefers-color-scheme: dark)') stand-in. */
function stubPrefersColorScheme(initial: boolean) {
  const listeners = new Set<ChangeListener>()
  const mql = {
    matches: initial,
    addEventListener(type: string, listener: ChangeListener) {
      if (type === 'change') listeners.add(listener)
    },
    removeEventListener(type: string, listener: ChangeListener) {
      if (type === 'change') listeners.delete(listener)
    },
    /** Simulates the user flipping the OS theme while the app runs. */
    flipTo(next: boolean) {
      mql.matches = next
      for (const listener of listeners) listener({ matches: next })
    },
  }
  vi.stubGlobal('matchMedia', () => mql)
  return mql
}

const themeAttribute = (): string | undefined => document.documentElement.dataset.theme

afterEach(() => {
  vi.unstubAllGlobals()
  delete document.documentElement.dataset.theme
})

describe('useOsTheme', () => {
  it.each([
    [false, 'light'],
    [true, 'dark'],
  ])('applies the OS theme on boot (prefers dark = %s → %s)', (prefersDark, expected) => {
    stubPrefersColorScheme(prefersDark)
    useOsTheme()
    expect(themeAttribute()).toBe(expected)
  })

  it('switches the theme when the OS setting changes at runtime — no reload', () => {
    // UI-021's live-switch criterion: the attribute (and with it every
    // theme token and dark: utility) flips on the matchMedia change
    // event, inside the already-running app.
    const mql = stubPrefersColorScheme(false)
    useOsTheme()
    expect(themeAttribute()).toBe('light')
    mql.flipTo(true)
    expect(themeAttribute()).toBe('dark')
    mql.flipTo(false)
    expect(themeAttribute()).toBe('light')
  })

  it('stops reacting after teardown (listener removed, attribute cleared)', () => {
    const mql = stubPrefersColorScheme(false)
    const teardown = useOsTheme()
    teardown()
    expect(themeAttribute()).toBeUndefined()
    mql.flipTo(true)
    expect(themeAttribute()).toBeUndefined()
  })

  it('is a no-op without matchMedia: the app keeps its light tokens', () => {
    vi.stubGlobal('matchMedia', undefined)
    useOsTheme()
    expect(themeAttribute()).toBeUndefined()
  })
})

describe('App wiring (the shell owns the theme bridge)', () => {
  const mountApp = () => mount(App, { global: { plugins: [i18n] } })

  it('mounts with data-theme already applied', () => {
    stubPrefersColorScheme(false)
    const wrapper = mountApp()
    expect(themeAttribute()).toBe('light')
    wrapper.unmount()
    expect(themeAttribute()).toBeUndefined()
  })

  it('mounts dark when the OS prefers dark, and follows a live OS switch', () => {
    const mql = stubPrefersColorScheme(true)
    const wrapper = mountApp()
    expect(themeAttribute()).toBe('dark')
    mql.flipTo(false)
    expect(themeAttribute()).toBe('light')
    wrapper.unmount()
  })
})
