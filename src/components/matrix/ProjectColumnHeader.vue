<script setup lang="ts">
// One project column header (T-364, MAIN-028, MAIN-033, UI-023): the
// internals of the mock-up's `th.c-proj` (docs/prototype/index.html
// 199–363) as a component — the color chip, the short label, the full
// name in the tooltip and an sr-only span, the per-status sub-line
// (icon + short text, or the short last-sync time on a clean column),
// and the header icon cluster app.js injects (edit, file preview, the
// "⋮" column menu — app.js 5079–5121; glyphs inlined as the ICONS
// markup facts). The "⋮" opens the ColumnMenu — the skeleton's
// minimum: open/close on click, outside click and Escape (scroll-close
// is T-184/T-355's); the menu actions write nothing. CONTAINER-
// AGNOSTIC: props in, one header cell out — no DOM-order traversal.
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import ColumnMenu from './ColumnMenu.vue'
import { anchorOf, type Anchor } from './floating'
import type { Project, ProjectStatus } from '../../dev/seed'

const props = defineProps<{
  /** The column's project (the header shows its label, color, status). */
  readonly project: Project
}>()

const { t } = useI18n()

/**
 * The short last-sync stamp of a clean column (MAIN-028: "19 Nov,
 * 09:14") derived from the seed's long display stamp ("19 Nov 2026,
 * 09:14 (UTC+3)") — the demo's format fact; the real-data card
 * (T-169's family) formats with Intl per UI-020 and replaces this.
 */
const shortSyncStamp = computed(() => {
  const match = /^(\d{1,2} [A-Za-z]{3}) \d{4}, (\d{2}:\d{2})/.exec(props.project.syncedAt)
  return match === null ? props.project.syncedAt : `${match[1]}, ${match[2]}`
})

/** The header tooltip: the full name, the status sentence, the stamp. */
const headerTitle = computed(() => {
  const name = props.project.name
  const stamp = props.project.syncedAt
  switch (props.project.status) {
    case 'ok':
    case 'dirty':
      return t('matrix.header.titleOk', { project: name, stamp })
    case 'external':
      return t('matrix.header.titleExternal', { project: name, stamp })
    case 'unavailable':
      return t('matrix.header.titleUnavailable', { project: name, stamp })
    case 'deleted':
      return t('matrix.header.titleDeleted', {
        project: name,
        file: props.project.baseFile,
        stamp,
      })
    default: {
      const exhaustive: never = props.project.status
      throw new Error(`unknown project status: ${String(exhaustive)}`)
    }
  }
})

/** The sub-line: a clean column shows the short last-sync stamp. */
const isOk = computed(() => props.project.status === 'ok')

/** The sub-line text per status (MAIN-028: icon + visible short text). */
const statusText = computed(() => {
  switch (props.project.status) {
    case 'dirty':
      return t('matrix.header.subDirty')
    case 'external':
      return t('matrix.header.subExternal')
    case 'unavailable':
      return t('matrix.header.subUnavailable')
    case 'deleted':
      return t('matrix.header.subDeleted')
    case 'ok':
      return shortSyncStamp.value
    default: {
      const exhaustive: never = props.project.status
      throw new Error(`unknown project status: ${String(exhaustive)}`)
    }
  }
})

// --- The skeleton's minimum menu behavior: the "⋮" toggles it. ---
const menuOpen = ref(false)
const menuAnchor = ref<Anchor>({ left: 0, right: 0, top: 0, bottom: 0 })

const toggleMenu = (event: MouseEvent): void => {
  if (menuOpen.value) {
    menuOpen.value = false
    return
  }
  menuAnchor.value = anchorOf(event.currentTarget as Element)
  menuOpen.value = true
}

const closeMenu = (): void => {
  menuOpen.value = false
}

// The menu items are real buttons; their actions are the bulk-operations
// card's behavior (§7.4) — the skeleton closes on select and writes nothing.
const onMenuSelect = (): void => {
  closeMenu()
}

// The status type guard for the template's icon branches (a ProjectStatus
// narrow, since only two statuses draw an icon).
const hasWarningIcon = (status: ProjectStatus): boolean =>
  status === 'external' || status === 'deleted'
</script>

<template>
  <th scope="col" class="c-proj" :title="headerTitle">
    <span class="proj-line">
      <span class="proj-color" :style="{ '--c': project.color }" aria-hidden="true"></span>
      <span class="proj-label">{{ project.label }}</span>
      <span class="proj-name visually-hidden">{{ project.name }}</span>
      <!-- Edit project (PRJ-008) — the settings-modal card's behavior. -->
      <button
        type="button"
        class="cell-icon cell-icon--edit proj-edit"
        :title="t('matrix.header.editProject', { project: project.name })"
        :aria-label="t('matrix.header.editProject', { project: project.name })"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path
            d="M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.03a.996.996 0 0 0 0-1.41l-2.34-2.34a.996.996 0 0 0-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z"
          />
        </svg>
      </button>
      <!-- File preview (MAIN-033) — the preview card's behavior. -->
      <button
        type="button"
        class="cell-icon proj-preview"
        :title="t('matrix.header.previewFile', { project: project.name })"
        :aria-label="t('matrix.header.previewFile', { project: project.name })"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path
            d="M14 2H6c-1.1 0-1.99.9-1.99 2L4 20c0 1.1.89 2 1.99 2H18c1.1 0 2-.9 2-2V8l-6-6zm2 16H8v-2h8v2zm0-4H8v-2h8v2zm-3-5V3.5L18.5 9H13z"
          />
        </svg>
      </button>
      <!-- Column actions menu, at the cell's right edge (UI-023). -->
      <button
        type="button"
        class="cell-icon col-menu-btn"
        :aria-haspopup="true"
        :aria-expanded="menuOpen"
        :title="t('matrix.header.columnActions', { project: project.name })"
        :aria-label="t('matrix.header.columnActions', { project: project.name })"
        @click="toggleMenu($event)"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path
            d="M12 8c1.1 0 2-.9 2-2s-.9-2-2-2-2 .9-2 2 .9 2 2 2zm0 2c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm0 6c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2z"
          />
        </svg>
      </button>
    </span>
    <span v-if="isOk" class="proj-sub">{{ statusText }}</span>
    <!-- Dirty: the dot, then the text (the mock-up's st-dirty line). -->
    <span v-else-if="project.status === 'dirty'" class="proj-sub st-dirty">
      <span class="dot" aria-hidden="true"></span>
      {{ statusText }}
    </span>
    <!-- External / deleted: the warning triangle (the mock-up's
         st-warn line). -->
    <span v-else-if="hasWarningIcon(project.status)" class="proj-sub st-warn">
      <svg class="ic" viewBox="0 0 16 16" aria-hidden="true">
        <path
          d="M8 2.6 14.4 13.4H1.6Z"
          fill="none"
          stroke="currentColor"
          stroke-width="1.5"
          stroke-linejoin="round"
        />
        <path d="M8 6.8v3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
        <circle cx="8" cy="11.6" r=".9" fill="currentColor" />
      </svg>
      {{ statusText }}
    </span>
    <!-- Unavailable: the circle-slash (the mock-up's st-off line). -->
    <span v-else class="proj-sub st-off">
      <svg class="ic" viewBox="0 0 16 16" aria-hidden="true">
        <circle cx="8" cy="8" r="5.4" fill="none" stroke="currentColor" stroke-width="1.5" />
        <path
          d="M4.6 11.4l6.8-6.8"
          stroke="currentColor"
          stroke-width="1.5"
          stroke-linecap="round"
        />
      </svg>
      {{ statusText }}
    </span>
    <ColumnMenu v-if="menuOpen" :anchor="menuAnchor" @close="closeMenu" @select="onMenuSelect" />
  </th>
</template>
