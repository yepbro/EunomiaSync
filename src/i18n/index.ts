// The i18n setup (T-142, spec UI-020; the per-feature split ruled in
// D-207 ruling 3): one vue-i18n instance in Composition API mode
// (`legacy: false`), one message catalog per locale. TEMPORARY SHAPE —
// during the frontend skeleton work the texts live in per-feature
// files under `src/locales/en/<feature>.json` (today `shell.json` for
// the main-window shell, `matrix.json` for the matrix family,
// `library.json` for the library family and `dev.json` for the
// DEV-only gallery) and are merged into the one catalog here at load
// time; the base `src/locales/en.json` keeps its T-142 content
// untouched. When the skeleton phase ends, the per-feature files are
// merged back into a single `en.json` and this header's note goes
// away. Today: `shell.json` (the main-window shell), `matrix.json`
// (the matrix family), `library.json` (the library family),
// `projects.json` (the projects family, T-366) and `dev.json` (the
// DEV-only gallery).
// Key convention (UI-020's `project.settings.title` example): a nested
// namespace per UI feature (`app.*` for the base catalog, `shell.*`
// for the main-window shell, `dev.*` for the gallery, later
// `project.*`, `library.*`, …), reached as a dotted path through
// `t('shell.toolbar.showHidden')`. The feature namespaces do not
// overlap, so the merge is a plain spread.
//
// Pluralization uses vue-i18n's pipe-separated plural forms — the
// CLDR plural-rule message format vue-i18n ships ("ICU plurals" in
// UI-020's wording), e.g. `shell.sync.counter`; counts interpolate
// through the composer's `n()` so numbers format per locale.
//
// Enforcement: user-visible raw text in templates is banned by the
// `eunomia/no-raw-ui-text` ESLint rule (see `./no-raw-ui-text.ts` and
// eslint.config.js) — every UI string goes through a key here. Key
// typos are caught by the catalog-coverage test in `./i18n.spec.ts`
// (every `t('…')` literal in `.vue` files must exist in the MERGED
// catalog this module builds): the `MessageSchema` generic below feeds
// vue-i18n's key AUTOCOMPLETE only — its `t()` accepts any string, so
// the generic does not reject unknown keys at compile time.

import { createI18n } from 'vue-i18n'
import en from '../locales/en.json'
import shell from '../locales/en/shell.json'
import matrix from '../locales/en/matrix.json'
import library from '../locales/en/library.json'
import projects from '../locales/en/projects.json'
import dev from '../locales/en/dev.json'

// The dev namespace serves only the DEV-only gallery (T-363): the
// statically-replaced `import.meta.env.DEV` guard makes the dev.json
// import dead code in a production build, so its strings stay out of
// the bundle with the gallery itself (the dist/ grep is the proof).
const devMessages = import.meta.env.DEV ? dev : {}

/** The merged English catalog: the base file plus the per-feature files. */
const catalog = { ...en, ...shell, ...matrix, ...library, ...projects, ...devMessages }

/** The message catalog type: nested feature namespaces, string leaves. */
export type MessageSchema = typeof catalog

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
  messages: { en: catalog },
})
