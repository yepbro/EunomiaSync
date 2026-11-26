/*
 * Seed about (T-367): the About window's product facts and the demo
 * release metadata of the frozen prototype (docs/prototype/app.js
 * ABOUT_NEXT_VERSION / ABOUT_RELEASE_CHANGES ~4934 and the dependency
 * licenses of #about-dialog's markup, index.html 3844–3934), reduced
 * to data. The values are the spec's constants (REL-004, UPD-005);
 * the dependency list is the demo's compact sample (the real list is
 * generated at build time — REL-004).
 *
 * The next version and the release changes are the UPDATE check's demo
 * metadata (UPD-001…003): the About window does not render them in
 * this skeleton (no check behavior — the update card's), they are
 * seeded for that card.
 */

/** The brand line's product name (the demo's static markup). */
export const productName = 'EunomiaSync'

/** The version the window shows (the demo's version chip and row). */
export const productVersion = '0.1.0-demo'

/** The bundle identifier (REL-004). */
export const appIdentifier = 'pro.yepbro.eunomiasync'

/** The author row's value (REL-004, the demo's rendered string). */
export const appAuthor = 'Den Brooks — denis@yepbro.pro'

/** The repository link (REL-004). */
export const repositoryUrl = 'https://github.com/yepbro/EunomiaSync'

/** The repository link's visible label (the demo's scheme-less form). */
export const repositoryLabel = 'github.com/yepbro/EunomiaSync'

/** The dependency-license demo list (REL-004: the real list is
 * generated at build time; the rows are the demo's rendered strings). */
export const dependencyLicenses: readonly string[] = [
  'rusqlite 0.32 — MIT/Apache-2.0',
  'tauri 2.x — MIT/Apache-2.0',
  'vue 3.x — MIT',
  'sha2 0.10 — MIT/Apache-2.0',
  'jiff 0.1 — MIT/Apache-2.0',
  'pulldown-cmark 0.12 — MIT',
]

/** The update check's demo metadata (UPD-002): the version the first
 * check of a session "finds". */
export const aboutNextVersion = '0.2.0'

/** The release changes shown with that version (UPD-002: the list
 * comes from the release metadata). */
export const aboutReleaseChanges: readonly string[] = [
  'Scheme B keeps BOM and line breaks',
  'Backup retention editing in Settings',
  'External-change watcher',
]
