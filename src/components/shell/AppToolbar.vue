<script setup lang="ts">
// The main-window toolbar (T-363, UI-001): brand, search, the project
// filter, the "Show hidden" toggle with its count, the selection chip
// and the bulk-assign button (hidden while nothing is selected), and
// the right cluster — "+ Add project", "+ New instruction", the
// icon-only Activity and Settings buttons, and "?" (Help). Markup and
// classes are the mock-up's (docs/prototype/index.html 67–158); the
// Activity/Settings glyphs are the ICONS markup facts app.js injects
// in the demo (app.js ~2590), inlined here as plain template SVG.
// No behavior: the controls render, the wiring belongs to later cards.
import { useI18n } from 'vue-i18n'
import type { Project } from '../../dev/seed'

defineProps<{
  /** The tracked projects: the filter options and (later) the columns. */
  readonly projects: readonly Project[]
  /** The concealed instruction count (LIB-005…007) next to the toggle. */
  readonly hiddenCount: number
  /** Selected row count; the chip and the bulk button stay hidden at 0. */
  readonly selectedCount: number
}>()

const { t, n } = useI18n()
</script>

<template>
  <header class="toolbar">
    <div class="brand">
      <span class="brand-mark" aria-hidden="true"></span>
      {{ t('app.title') }}
    </div>

    <label class="visually-hidden" for="search">{{ t('shell.toolbar.searchLabel') }}</label>
    <input
      id="search"
      class="search"
      type="search"
      :placeholder="t('shell.toolbar.searchPlaceholder')"
      :title="t('shell.toolbar.searchTitle')"
      autocomplete="off"
    />

    <label class="visually-hidden" for="proj-filter">{{ t('shell.toolbar.filterLabel') }}</label>
    <select id="proj-filter" class="filter">
      <option selected value="all">{{ t('shell.toolbar.filterAll') }}</option>
      <option v-for="project in projects" :key="project.name" :value="project.name">
        {{ project.name }}
      </option>
    </select>

    <label class="toggle">
      <input id="show-hidden" type="checkbox" />
      {{ t('shell.toolbar.showHidden') }}
      <span class="toggle-count" :title="t('shell.toolbar.hiddenCountTitle')">
        {{ hiddenCount }}
      </span>
    </label>

    <span class="spacer"></span>

    <span class="selected-chip" :hidden="selectedCount === 0">
      {{ t('shell.toolbar.selectedChip', { count: n(selectedCount) }, selectedCount) }}
    </span>

    <!-- Bulk assignment (§7.4 UI bulk operations, UI-010): appears with
         a row selection. Hidden in this skeleton: there is no selection
         state yet (the matrix card wires it). The id is the reference
         markup fact — #bulk-assign-btn[hidden] (matrix-bulk.css)
         overrides .btn's display, so `hidden` really hides the button. -->
    <button id="bulk-assign-btn" type="button" class="btn btn-ghost" :hidden="selectedCount === 0">
      {{ t('shell.toolbar.bulkAssign') }}
    </button>

    <!-- Activity (§7.7 UI row) and Settings (DEF-001…004): icon ghost
         buttons in the right cluster, per the spec toolbar sketch.
         Icon-only keeps the cluster inside the UI-001 minimum width of
         960px. The other Settings entry point is Ctrl/Cmd + , (§11.3). -->
    <button
      type="button"
      class="btn btn-ghost btn-icon"
      :title="t('shell.toolbar.activityTitle')"
      :aria-label="t('shell.toolbar.activityLabel')"
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path
          d="M11.99 2C6.47 2 2 6.48 2 12s4.47 10 9.99 10C17.52 22 22 17.52 22 12S17.52 2 11.99 2zM12 20c-4.42 0-8-3.58-8-8s3.58-8 8-8 8 3.58 8 8-3.58 8-8 8zm.5-13H11v6l5.25 3.15.75-1.23-4.5-2.67z"
        />
      </svg>
    </button>

    <button
      type="button"
      class="btn btn-ghost btn-icon"
      :title="t('shell.toolbar.settingsTitle')"
      :aria-label="t('shell.toolbar.settingsLabel')"
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path
          d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58a.49.49 0 0 0 .12-.61l-1.92-3.32a.488.488 0 0 0-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54a.484.484 0 0 0-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58a.49.49 0 0 0-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z"
        />
      </svg>
    </button>
    <button type="button" class="btn btn-ghost" :title="t('shell.toolbar.addProjectTitle')">
      {{ t('shell.toolbar.addProject') }}
    </button>
    <button type="button" class="btn btn-ghost" :title="t('shell.toolbar.newInstructionTitle')">
      {{ t('shell.toolbar.newInstruction') }}
    </button>

    <!-- Help (HLP-001): ghost button, the last toolbar button per the
         §11.1 sketch (…[Activity] [Settings] [?]); the other entry
         point is F1 (§11.3) -->
    <button
      type="button"
      class="btn btn-ghost"
      :title="t('shell.toolbar.helpTitle')"
      :aria-label="t('shell.toolbar.helpLabel')"
    >
      {{ t('shell.toolbar.help') }}
    </button>
  </header>
</template>
