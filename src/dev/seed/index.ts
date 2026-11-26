/*
 * The seed bundle (T-363): the one import point for the mock data —
 * only containers may import it (App.vue, DevGallery.vue); components
 * take props. The family cards T-364…T-367 extend the modules here
 * with their screens' data (activity, help, scan results, sync
 * summary, archived, exclusions, about) — do not port those now.
 *
 * The bundle reaches the app through a DEV-only dynamic import
 * (App.vue), so the whole directory is absent from the production
 * bundle; the build's dist/ grep is the proof (T-363's record in
 * DECISIONS.md).
 */

export * from './types'
export * from './projects'
export * from './library'
export * from './assignments'
export * from './settings'
