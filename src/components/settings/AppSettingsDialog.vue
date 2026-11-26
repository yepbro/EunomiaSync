<script setup lang="ts">
// The app Settings window (T-367, DEF-001…004, DEF-005; §6.5 keys and
// ranges): the mock-up's #settings-dialog (docs/prototype/index.html
// 3368–3621) on the T-363 dialog contract — BaseModal (size lg) +
// defineModel<boolean>('open'). Five vertical sections in one
// scrollable body (no tabs, unlike the project settings modal), one
// component each (the T-367 structure). There is NO Save button:
// the static status line says the settings are applied immediately
// (DEF-001). Everything visible is derived from props: the §6.5 seed
// values, the §6.5 ranges, and the one field rendered in its
// range-error state (aria-invalid, the inline error, the typed
// out-of-range value shown while the stored setting keeps the previous
// value — DEF-001…004). No behavior: no field application, no
// validation, no exclusions editing, no link handling — the controls
// are the mock-up's inert controls and the behavior is the settings
// card's (T-199's family).
import { useId } from 'vue'
import { useI18n } from 'vue-i18n'
import BaseModal from '../ui/BaseModal.vue'
import SettingsDefaultsSection from './SettingsDefaultsSection.vue'
import SettingsBackupsSection from './SettingsBackupsSection.vue'
import SettingsScanningSection from './SettingsScanningSection.vue'
import SettingsLogsSection from './SettingsLogsSection.vue'
import SettingsUpdatesSection from './SettingsUpdatesSection.vue'
import type { Settings, SettingNumberField, SettingNumberRange } from '../../dev/seed'

const props = defineProps<{
  /** The stored settings the controls carry (the seed's §6.5 values). */
  readonly values: Settings
  /** The §6.5 closed range of each number field (DEF-001…004). */
  readonly ranges: Record<SettingNumberField, SettingNumberRange>
  /** The one field in its range-error state with the typed
   * out-of-range value its input shows — null is the clean state. */
  readonly rangeError: {
    readonly field: SettingNumberField
    readonly value: number
  } | null
  /** The base-file options (the mock-up's static select options). */
  readonly baseFiles: readonly string[]
}>()

const { t } = useI18n()

/** The dialog contract (T-363): v-model:open drives the BaseModal. */
const open = defineModel<boolean>('open', { default: false })

const titleId = useId()

/** The error state of one number field, or null when the field is
 * clean (the dialog maps the single demo error onto the sections). */
const errorOf = (field: SettingNumberField): { readonly value: number } | null =>
  props.rangeError?.field === field ? { value: props.rangeError.value } : null
</script>

<template>
  <BaseModal v-model:open="open" size="lg" :labelled-by="titleId">
    <form method="dialog" novalidate>
      <h2 :id="titleId">{{ t('settings.title') }}</h2>

      <div class="ps-body set-body">
        <SettingsDefaultsSection :defaults="props.values.default" :base-files="props.baseFiles" />
        <SettingsBackupsSection
          :value="props.values.backup.retention"
          :range="props.ranges.backupRetention"
          :error="errorOf('backupRetention')"
        />
        <SettingsScanningSection
          :depth="props.values.scan.maxDepth"
          :range="props.ranges.scanDepth"
          :error="errorOf('scanDepth')"
          :exclude-names="props.values.scan.excludeNames"
        />
        <SettingsLogsSection
          :revisions="props.values.history.maxRevisions"
          :revisions-range="props.ranges.historyRevisions"
          :revisions-error="errorOf('historyRevisions')"
          :log-entries="props.values.log.maxEntries"
          :log-entries-range="props.ranges.logEntries"
          :log-entries-error="errorOf('logEntries')"
          :log-days="props.values.log.maxDays"
          :log-days-range="props.ranges.logDays"
          :log-days-error="errorOf('logDays')"
        />
        <SettingsUpdatesSection :check-on-start="props.values.update.checkOnStart" />
      </div>

      <p class="ps-hint set-status">{{ t('settings.statusLine') }}</p>

      <div class="modal-actions">
        <button value="close" class="btn btn-ghost">{{ t('settings.close') }}</button>
      </div>
    </form>
  </BaseModal>
</template>
