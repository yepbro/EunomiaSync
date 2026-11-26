<script setup lang="ts">
// The main-window shell (T-363, UI-001/MAIN-010/MAIN-025; the matrix
// body since T-364): the toolbar, the matrix frame and the sync panel,
// laid out as the mock-up's `.app` column — toolbar and syncbar
// `flex: none`, the matrix region scrolling between them, so the sync
// panel is visible at every window size (MAIN-025; T-148's absorbed
// criterion).
//
// No IPC here: the skeleton phase renders on mock data, and the seed
// (src/dev/seed) reaches the shell ONLY through the DEV-guarded
// dynamic import below — in a production build the guard is statically
// false, the import is dead code, and the whole src/dev/ tree stays
// out of the bundle (the build's dist/ grep is the standing proof;
// T-212 wires the real `app_info`/`projects_list` data later).
//
// All user-visible strings come from the i18n catalog (UI-020); the
// shell's copy lives in src/locales/en/shell.json and the matrix's in
// src/locales/en/matrix.json (the temporary per-feature split, D-207
// ruling 3). The UI-021 theme bridge stays: useOsTheme() wires the OS
// light/dark setting to <html data-theme>.
import { defineAsyncComponent, onMounted, ref, shallowRef } from 'vue'
import { useI18n } from 'vue-i18n'
import { useOsTheme } from './assets/theme'
import AppToolbar from './components/shell/AppToolbar.vue'
import MatrixFrame from './components/shell/MatrixFrame.vue'
import SyncPanel from './components/shell/SyncPanel.vue'
import type { AssignmentMatrix, LibraryRow, Project } from './dev/seed'

// The DEV-only gallery: dynamic import + the static DEV guard keep it
// (and its registry) out of the production bundle.
const DevGallery = import.meta.env.DEV
  ? defineAsyncComponent(() => import('./dev/DevGallery.vue'))
  : null

useOsTheme()

const { t } = useI18n()

/** The tracked projects (the filter options; the columns per seed). */
const projects = shallowRef<readonly Project[]>([])
/** The concealed instructions (LIB-005…007) next to "Show hidden". */
const hiddenCount = ref(0)
/** The sync panel's last-sync stamp and unsynced-changes counter. */
const lastSyncAt = ref('')
const dirtyProjects = ref(0)
/** The library rows in document order (the matrix body, T-364). */
const library = shallowRef<readonly LibraryRow[]>([])
/** The assignment cells, keyed by instruction then project name. */
const assignments = shallowRef<AssignmentMatrix>({})

onMounted(async () => {
  if (!import.meta.env.DEV) return
  const seed = await import('./dev/seed')
  projects.value = seed.projects
  hiddenCount.value = seed.hiddenInstructionCount
  lastSyncAt.value = seed.lastSuccessfulSync
  dirtyProjects.value = seed.dirtyProjectCount
  library.value = seed.library
  assignments.value = seed.assignments
})

// Row selection is the matrix card's behavior (bulk assignment, UI-010);
// the skeleton renders the toolbar's selection cluster hidden at rest.
const SELECTED_ROWS = 0
</script>

<template>
  <div class="app">
    <h1 class="visually-hidden">{{ t('app.title') }}</h1>

    <AppToolbar :projects="projects" :hidden-count="hiddenCount" :selected-count="SELECTED_ROWS" />

    <!-- The seed's library rows and assignment cells (T-364): the
         frame renders them in its tbody; with an empty library the
         UI-012 no-instructions empty state shows. -->
    <MatrixFrame :projects="projects" :rows="library" :assignments="assignments" />

    <SyncPanel :last-sync-at="lastSyncAt" :dirty-projects="dirtyProjects" />

    <DevGallery v-if="DevGallery !== null" />
  </div>
</template>
