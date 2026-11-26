<script setup lang="ts">
// The project settings window (T-366, PRJ-008…012, 021, BKP-004/012):
// the mock-up's #project-settings-dialog (docs/prototype/index.html
// 2874–3126) on the T-363 dialog contract — BaseModal (size lg) +
// defineModel<boolean>('open'). The reference's two tabs (General /
// Backups) are ARIA tabs; the active tab is the skeleton's ONLY local
// UI state (click switching; the arrow-key roving focus is a feature
// card's). The panels compose the three tab components: the General
// panel carries the fields (SettingsGeneralTab) and the sync-settings
// section (SettingsSchemeTab) — the mock-up renders both inside the
// General panel; the Backups panel the copies table
// (SettingsBackupsTab). Every field carries the seed project's value
// through props; nothing here edits, validates, saves, copies or
// archives — the save/validation flow is T-158's family, the copy
// feedback T-160's, the archive flow T-165's.
import { ref, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import BaseModal from '../ui/BaseModal.vue'
import SettingsGeneralTab from './SettingsGeneralTab.vue'
import SettingsSchemeTab from './SettingsSchemeTab.vue'
import SettingsBackupsTab from './SettingsBackupsTab.vue'
import type { BackupRow, Project } from '../../dev/seed'

const props = defineProps<{
  /** The project the window shows (the seed project a caller picks). */
  readonly project: Project
  /** The palette the color fieldset offers (the demo's PS_PALETTE). */
  readonly palette: readonly (readonly [hex: string, name: string])[]
  /** The base-file options (the mock-up's static select options). */
  readonly baseFiles: readonly string[]
  /** The Backups tab's rows (BKP-004). */
  readonly backups: readonly BackupRow[]
}>()

const { t } = useI18n()

/** The dialog contract (T-363): v-model:open drives the BaseModal. */
const open = defineModel<boolean>('open', { default: false })

const titleId = useId()
const generalTabId = useId()
const backupsTabId = useId()
const generalPanelId = useId()
const backupsPanelId = useId()

/** Which panel is open; General is the mock-up's initial tab. */
const activeTab = ref<'general' | 'backups'>('general')
</script>

<template>
  <BaseModal v-model:open="open" size="lg" :labelled-by="titleId">
    <form method="dialog" novalidate>
      <h2 :id="titleId">{{ t('projects.settings.title') }}</h2>

      <div class="ps-tabs" role="tablist" :aria-label="t('projects.settings.tabsLabel')">
        <button
          :id="generalTabId"
          type="button"
          class="ps-tab"
          role="tab"
          :aria-selected="activeTab === 'general'"
          :aria-controls="generalPanelId"
          :tabindex="activeTab === 'general' ? 0 : -1"
          @click="activeTab = 'general'"
        >
          {{ t('projects.settings.tabGeneral') }}
        </button>
        <button
          :id="backupsTabId"
          type="button"
          class="ps-tab"
          role="tab"
          :aria-selected="activeTab === 'backups'"
          :aria-controls="backupsPanelId"
          :tabindex="activeTab === 'backups' ? 0 : -1"
          @click="activeTab = 'backups'"
        >
          {{ t('projects.settings.tabBackups') }}
        </button>
      </div>

      <div class="ps-body">
        <div
          :id="generalPanelId"
          class="ps-panel"
          role="tabpanel"
          :aria-labelledby="generalTabId"
          :hidden="activeTab !== 'general'"
        >
          <SettingsGeneralTab :project="props.project" :palette="props.palette" />
          <SettingsSchemeTab :project="props.project" :base-files="props.baseFiles" />
        </div>
        <div
          :id="backupsPanelId"
          class="ps-panel"
          role="tabpanel"
          :aria-labelledby="backupsTabId"
          :hidden="activeTab !== 'backups'"
        >
          <SettingsBackupsTab :backups="props.backups" />
        </div>
      </div>

      <div class="ps-danger">
        <button type="button" class="btn btn-danger">
          {{ t('projects.settings.archiveBtn') }}
        </button>
        <p class="ps-hint">{{ t('projects.settings.archiveHint') }}</p>
      </div>

      <div class="modal-actions">
        <button value="cancel" class="btn btn-ghost">{{ t('projects.settings.cancel') }}</button>
        <button value="save" class="btn btn-primary">{{ t('projects.settings.save') }}</button>
      </div>
    </form>
  </BaseModal>
</template>
