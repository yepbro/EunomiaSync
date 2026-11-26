<script setup lang="ts">
// The bottom sync panel (T-363, MAIN-024…027): pinned outside the
// scroll area — `flex: none` in the `.app` column — with the Run sync
// button, the run-progress slot (hidden while nothing runs: the
// sync-behavior card fills and styles it), the last-sync stamp and the
// unsynced-changes counter. Markup and classes are the mock-up's
// (docs/prototype/index.html 2509–2548); no behavior in this skeleton.
import { useI18n } from 'vue-i18n'

defineProps<{
  /** The demo's last-sync stamp (the newest per-project syncedAt). */
  readonly lastSyncAt: string
  /** Projects with unsynced changes: the badge and the counter line. */
  readonly dirtyProjects: number
}>()

const { t, n } = useI18n()
</script>

<template>
  <footer class="syncbar">
    <button type="button" class="btn btn-primary" :title="t('shell.sync.runSyncTitle')">
      {{ t('shell.sync.runSync') }}
    </button>

    <!-- Run progress (MAIN-030): shown by the sync-behavior card while
         a run ticks; "k of N" is a polite live region, the spinner is
         decorative and drops its animation under reduced motion. -->
    <span class="sync-progress" aria-live="polite" hidden>
      <span class="sync-spinner" aria-hidden="true"></span>
      <span></span>
    </span>

    <span class="sync-last">
      {{ t('shell.sync.lastSyncLabel') }}
      <strong>{{ lastSyncAt }}</strong>
    </span>

    <span class="spacer"></span>

    <button type="button" class="sync-counter" :title="t('shell.sync.counterTitle')">
      <span class="badge" aria-hidden="true">{{ dirtyProjects }}</span>
      <span>{{ t('shell.sync.counter', { count: n(dirtyProjects) }, dirtyProjects) }}</span>
    </button>
  </footer>
</template>
