// The i18n setup (T-142, spec UI-020): one vue-i18n instance in Composition
// API mode (`legacy: false`), one message catalog per locale under
// `src/locales/` — today `en.json`, the app's single locale (NFR-004…006:
// the UI language is English; backend error texts arrive as `message_key`
// + `params` and will resolve against this same catalog).
//
// Key convention (UI-020's `project.settings.title` example): a nested
// namespace per UI feature (`app.*` for the shell, later `project.*`,
// `library.*`, …), reached as a dotted path through `t('app.title')`.
//
// Pluralization uses vue-i18n's pipe-separated plural forms — the CLDR
// plural-rule message format vue-i18n ships ("ICU plurals" in UI-020's
// wording), e.g. `app.trackedProjects`; counts interpolate through the
// composer's `n()` so numbers format per locale (the working example
// renders `t('app.trackedProjects', { count: n(projectCount) },
// projectCount)` — the explicit numeric argument selects the plural
// form, `n()` formats the interpolated count: "1,000 projects tracked").
//
// Enforcement: user-visible raw text in templates is banned by the
// `eunomia/no-raw-ui-text` ESLint rule (see `./no-raw-ui-text.ts` and
// eslint.config.js) — every UI string goes through a key here. Key
// typos are caught by the catalog-coverage test in `./i18n.spec.ts`
// (every `t('…')` literal in `.vue` files must exist in the catalog):
// the `MessageSchema` generic below feeds vue-i18n's key AUTOCOMPLETE
// only — its `t()` accepts any string, so the generic does not reject
// unknown keys at compile time.

import { createI18n } from 'vue-i18n'
import en from '../locales/en.json'

/** The message catalog type: nested feature namespaces, string leaves. */
export type MessageSchema = typeof en

/** The app's single locale (message catalogs under `src/locales/`). */
export type AppLocale = 'en'

/**
 * The one vue-i18n instance. Composition API mode only (`legacy: false`):
 * components use `useI18n()`; the global `$t` injection is not relied on.
 */
export const i18n = createI18n<[MessageSchema], AppLocale>({
  legacy: false,
  locale: 'en',
  fallbackLocale: 'en',
  messages: { en },
})
