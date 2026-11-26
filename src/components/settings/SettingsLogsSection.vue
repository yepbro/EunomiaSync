<script setup lang="ts">
// The Settings window's "History and logs" section (T-367, DEF-001…004):
// the mock-up's fourth set-body section (docs/prototype/index.html
// 3517–3567) — the three §6.5 limits side by side in the .ps-grid-3
// grid (versions per entity, log entries, log days), each a number
// with its hint and inline error slot. Each field renders in its
// range-error state when its `error` prop is non-null (the demo's
// error-state facts); the ONE demo error state lands on the Backups
// field, so all three render clean here. No behavior: no application.
import { useId } from 'vue'
import { useI18n } from 'vue-i18n'
import type { SettingNumberRange } from '../../dev/seed'

const props = defineProps<{
  /** The stored versions-per-entity value (history.maxRevisions). */
  readonly revisions: number
  readonly revisionsRange: SettingNumberRange
  readonly revisionsError: { readonly value: number } | null
  /** The stored log-entries value (log.maxEntries). */
  readonly logEntries: number
  readonly logEntriesRange: SettingNumberRange
  readonly logEntriesError: { readonly value: number } | null
  /** The stored log-days value (log.maxDays). */
  readonly logDays: number
  readonly logDaysRange: SettingNumberRange
  readonly logDaysError: { readonly value: number } | null
}>()

const { t } = useI18n()

const revisionsId = useId()
const revisionsHintId = useId()
const revisionsErrorId = useId()
const entriesId = useId()
const entriesHintId = useId()
const entriesErrorId = useId()
const daysId = useId()
const daysHintId = useId()
const daysErrorId = useId()
const titleId = useId()
</script>

<template>
  <section class="ps-section" :aria-labelledby="titleId">
    <h3 :id="titleId" class="ps-section-title">{{ t('settings.logsTitle') }}</h3>

    <div class="ps-grid ps-grid-3">
      <div class="ps-field">
        <label class="field-label" :for="revisionsId">{{ t('settings.revisionsLabel') }}</label>
        <input
          :id="revisionsId"
          class="ps-input set-number"
          type="number"
          :min="props.revisionsRange.min"
          :max="props.revisionsRange.max"
          step="1"
          :value="props.revisionsError === null ? props.revisions : props.revisionsError.value"
          :aria-invalid="props.revisionsError !== null"
          :aria-describedby="`${revisionsHintId} ${revisionsErrorId}`"
        />
        <p :id="revisionsHintId" class="ps-hint">{{ t('settings.revisionsHint') }}</p>
        <p :id="revisionsErrorId" class="ps-error" :hidden="props.revisionsError === null">
          {{ t('settings.error.historyRevisions') }}
        </p>
      </div>
      <div class="ps-field">
        <label class="field-label" :for="entriesId">{{ t('settings.logEntriesLabel') }}</label>
        <input
          :id="entriesId"
          class="ps-input set-number"
          type="number"
          :min="props.logEntriesRange.min"
          :max="props.logEntriesRange.max"
          step="1"
          :value="props.logEntriesError === null ? props.logEntries : props.logEntriesError.value"
          :aria-invalid="props.logEntriesError !== null"
          :aria-describedby="`${entriesHintId} ${entriesErrorId}`"
        />
        <p :id="entriesHintId" class="ps-hint">{{ t('settings.logEntriesHint') }}</p>
        <p :id="entriesErrorId" class="ps-error" :hidden="props.logEntriesError === null">
          {{ t('settings.error.logEntries') }}
        </p>
      </div>
      <div class="ps-field">
        <label class="field-label" :for="daysId">{{ t('settings.logDaysLabel') }}</label>
        <input
          :id="daysId"
          class="ps-input set-number"
          type="number"
          :min="props.logDaysRange.min"
          :max="props.logDaysRange.max"
          step="1"
          :value="props.logDaysError === null ? props.logDays : props.logDaysError.value"
          :aria-invalid="props.logDaysError !== null"
          :aria-describedby="`${daysHintId} ${daysErrorId}`"
        />
        <p :id="daysHintId" class="ps-hint">{{ t('settings.logDaysHint') }}</p>
        <p :id="daysErrorId" class="ps-error" :hidden="props.logDaysError === null">
          {{ t('settings.error.logDays') }}
        </p>
      </div>
    </div>
  </section>
</template>
