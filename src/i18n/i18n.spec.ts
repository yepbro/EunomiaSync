import { describe, expect, it } from 'vitest'
import { i18n } from './index'

// The message-catalog contract (UI-020): nested feature keys, named
// interpolation, and vue-i18n pipe plurals (its CLDR plural-rule message
// format — "ICU plurals" in the spec's wording). With `legacy: false`
// the global instance is a Composer, the same object components reach
// through `useI18n()`.
const { t, n, te } = i18n.global

describe('i18n message catalog (en)', () => {
  it('resolves nested feature-namespace keys', () => {
    expect(t('app.title')).toBe('EunomiaSync')
    expect(t('app.logoAlt.vite')).toBe('Vite logo')
  })

  it('interpolates named parameters', () => {
    expect(t('app.versionLine', { version: '0.1.0' })).toBe('v0.1.0')
  })

  it('pluralizes the zero/one/other forms (1/2/5 and 0)', () => {
    expect(t('app.trackedProjects', 0)).toBe('no projects tracked yet')
    expect(t('app.trackedProjects', 1)).toBe('1 project tracked')
    expect(t('app.trackedProjects', 2)).toBe('2 projects tracked')
    expect(t('app.trackedProjects', 5)).toBe('5 projects tracked')
  })

  it('formats interpolated counts with the locale number format', () => {
    // The count line interpolates `n(count)` — 1000 must read "1,000",
    // not "1000" (UI-020's locale-aware wording; spec scale is 2000).
    expect(t('app.trackedProjects', { count: n(1000) }, 1000)).toBe('1,000 projects tracked')
  })
})

// A missing key renders as the literal key string (vue-i18n only warns),
// and neither vue-i18n's generics nor vue-tsc reject an unknown key —
// so this is the typo gate: every `t('…')` string literal used in a
// `.vue` template must exist in the catalog. Intentionally a raw-source
// scan (comments are stripped first) rather than an AST walk: it covers
// template and `<script setup>` call sites with zero parser plumbing.
// Known false negatives, accepted: an aliased `t` (`const { t: tr }`)
// and dynamically built keys are invisible to the regex. Dynamically
// built keys — keys reaching `t()` through a table
// (`t(table[member])`) — are covered at compile time instead: every
// such table is typed `satisfies Record<Enum, CatalogKey>` (the type
// lives in src/i18n/index.ts; ActivityDialog's date-presets list, the
// one non-Record holder, types its key field `CatalogKey`), so a typo
// or a renamed key there fails `vue-tsc`; a planted typo in a literal
// `t('…')` is caught by this scan either way.
const T_CALL_LITERAL = /\bt\(\s*(['"])([^'"\n]+)\1/g

/** Line/block/HTML comments, where a `t('…')` mention is not a usage. */
const COMMENTS = /<!--[\s\S]*?-->|\/\*[\s\S]*?\*\/|^\s*\/\/.*$/gm

// All .vue templates under src/, as raw sources (Vite glob — no Node
// types needed; covers future components automatically).
const VUE_SOURCES: Record<string, string> = import.meta.glob('../**/*.vue', {
  query: '?raw',
  import: 'default',
  eager: true,
})

function tLiteralsIn(source: string): string[] {
  const keys: string[] = []
  const withoutComments = source.replace(COMMENTS, '')
  for (const match of withoutComments.matchAll(T_CALL_LITERAL)) keys.push(match[2])
  return keys
}

describe('catalog coverage (every t() literal in .vue templates exists)', () => {
  it('scans the .vue templates and finds at least one t() literal', () => {
    const keys = Object.values(VUE_SOURCES).flatMap((source) => tLiteralsIn(source))
    expect(Object.keys(VUE_SOURCES).length).toBeGreaterThan(0)
    expect(keys.length).toBeGreaterThan(0)
  })

  it('resolves every t() literal against the catalog', () => {
    const missing: string[] = []
    for (const [file, source] of Object.entries(VUE_SOURCES)) {
      for (const key of tLiteralsIn(source)) {
        if (!te(key)) missing.push(`${key} (used in ${file})`)
      }
    }
    expect(missing).toEqual([])
  })
})
