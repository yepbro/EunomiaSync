/*
 * The seed bundle (T-363): the one import point for the mock data —
 * only containers may import it (App.vue, DevGallery.vue); components
 * take props. The family cards T-364…T-367 extend the modules here
 * with their screens' data (T-364 matrix, T-365 history/preview,
 * T-366 scan/archived/exclusions/backups/file-preview, T-367 the sync
 * summary, activity, help and about).
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
/* The library family's additions (T-365): the history panel's demo
   revisions and the static Preview sample. */
export * from './history'
export * from './preview'
/* The projects family's additions (T-366): the scan walk's end state
   and its three result groups, the archived projects, the exclusions,
   the Backups-tab rows and the file-preview sample. */
export * from './scan'
export * from './archived'
export * from './exclusions'
export * from './backups'
export * from './file-preview'
/* The service screens' additions (T-367): the sync summary and pending
   rows, the activity log, the help articles and the About facts. */
export * from './sync'
export * from './activity'
export * from './help'
export * from './about'
