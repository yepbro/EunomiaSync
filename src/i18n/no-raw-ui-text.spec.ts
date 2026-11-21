import { describe, expect, it } from 'vitest'
import { RuleTester } from 'eslint'
import vueParser from 'vue-eslint-parser'
import noRawUiText from './no-raw-ui-text'

// The UI-020 gate, pinned red/green: a raw UI string in a template must
// FAIL lint. `vue/no-raw-text` does not exist in eslint-plugin-vue 10.x,
// so the rule is written in-repo (see ./no-raw-ui-text.ts) — which makes
// this proof of fire mandatory: a rule that silently stops matching is a
// gate that no longer exists.

// Route RuleTester's runner hooks onto Vitest so each case reports
// individually instead of aborting on the first failure.
RuleTester.describe = (name, fn) => describe(name, fn)
RuleTester.it = (name, fn) => it(name, fn)

const tester = new RuleTester({
  languageOptions: {
    parser: vueParser,
    parserOptions: { sourceType: 'module' },
  },
})

tester.run('eunomia/no-raw-ui-text', noRawUiText, {
  valid: [
    // The sanctioned form: copy through keys and interpolations.
    { code: `<template><p>{{ t('app.title') }}</p></template>` },
    { code: '<template><p>{{ version }}</p></template>' },
    { code: `<template><img :alt="t('app.logoAlt.vite')" /></template>` },
    // Expressions with no bare literal copy are not flagged: comparisons,
    // filters and identifiers are not copy.
    {
      code: `<template><p>{{ items.filter((x) => x.name !== 'x').length }}</p></template>`,
    },
    { code: `<template><p>{{ x === 'en' ? t('app.title') : t('app.tagline') }}</p></template>` },
    { code: '<template><p>{{ `${version}` }}</p></template>' },
    // Punctuation/symbols without letters or digits are formatting.
    { code: `<template><p>— {{ t('app.tagline') }}…</p></template>` },
    // Attribute literals the user never sees are not copy.
    { code: `<template><a href="https://vite.dev">{{ t('app.title') }}</a></template>` },
    { code: '<template><div class="container"></div></template>' },
    // `href`/`class` are machine-facing: literals there stay unflagged.
    { code: '<template><a href="https://vite.dev" class="link"></a></template>' },
    // `value` is data on text inputs and options — only submit-style
    // inputs render it as the control's label.
    { code: '<template><input value="form data" /></template>' },
    { code: `<template><option value="a">{{ t('app.title') }}</option></template>` },
    // The exemption valve: ignoreText / ignorePattern (P3-2 pin).
    { code: '<template><p>·</p></template>', options: [{ ignoreText: ['·'] }] },
    { code: '<template><p>⌘K</p></template>', options: [{ ignorePattern: '^⌘' }] },
  ],
  invalid: [
    // THE red example: a raw UI string in a template text node.
    {
      code: '<template><p>EunomiaSync</p></template>',
      errors: [{ messageId: 'rawUiText' }],
    },
    // A raw copy prefix around an interpolation is still raw UI text.
    {
      code: '<template><p>v{{ version }}</p></template>',
      errors: [{ messageId: 'rawUiText' }],
    },
    // User-facing attribute literals are copy too (screen readers read them).
    {
      code: '<template><img alt="Vite logo" /></template>',
      errors: [{ messageId: 'rawUiText' }],
    },
    // A string literal as a whole interpolation is copy without a key.
    {
      code: `<template><p>{{ 'Save' }}</p></template>`,
      errors: [{ messageId: 'rawUiText' }],
    },
    // …and so is a literal bound to a user-facing attribute.
    {
      code: `<template><img :alt="'Vite logo'" /></template>`,
      errors: [{ messageId: 'rawUiText' }],
    },
    // Concatenated literals and template literals are copy too.
    {
      code: `<template><p>{{ 'Sa' + 've' }}</p></template>`,
      errors: [{ messageId: 'rawUiText' }],
    },
    {
      code: '<template><p>{{ `Save` }}</p></template>',
      errors: [{ messageId: 'rawUiText' }],
    },
    // Bare copy appended to a keyed message bypasses nothing.
    {
      code: `<template><p>{{ t('app.title') + ' — Beta' }}</p></template>`,
      errors: [{ messageId: 'rawUiText' }],
    },
    // A copy prefix inside a template literal is the `v{{ version }}`
    // violation in expression form.
    {
      code: '<template><p>{{ `v${version}` }}</p></template>',
      errors: [{ messageId: 'rawUiText' }],
    },
    // `value` on a submit-style input renders as the button label.
    {
      code: '<template><input type="submit" value="Search" /></template>',
      errors: [{ messageId: 'rawUiText' }],
    },
    // …in any tag casing (`rawName` keeps it, the check must not).
    {
      code: '<template><INPUT type="submit" value="Search" /></template>',
      errors: [{ messageId: 'rawUiText' }],
    },
    // An ignorePattern that matches nothing exempts nothing.
    {
      code: '<template><p>Save</p></template>',
      options: [{ ignorePattern: '^$' }],
      errors: [{ messageId: 'rawUiText' }],
    },
  ],
})

// The rule must actually be wired into the flat config — a tested rule
// that eslint.config.js forgot to enable gates nothing.
//
// @ts-expect-error the JS config module has no declaration file (allowJs
// is off); its shape is asserted at runtime below.
import eslintConfig from '../../eslint.config.js'

describe('eslint.config.js wiring', () => {
  it('enables eunomia/no-raw-ui-text as an error for .vue templates', () => {
    const wiring = (eslintConfig as Array<Record<string, unknown>>).filter(
      (entry) =>
        typeof entry === 'object' &&
        entry !== null &&
        typeof entry.rules === 'object' &&
        entry.rules !== null &&
        'eunomia/no-raw-ui-text' in entry.rules,
    )
    expect(wiring.length).toBeGreaterThan(0)
    const entry = wiring[wiring.length - 1] as {
      files?: unknown
      rules: Record<string, unknown>
    }
    expect(entry.files).toContain('**/*.vue')
    expect(entry.rules['eunomia/no-raw-ui-text']).toBe('error')
  })
})
