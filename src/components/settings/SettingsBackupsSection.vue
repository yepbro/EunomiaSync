<script setup lang="ts">
// The Settings window's "Backups" section (T-367, DEF-001…004): the
// mock-up's second set-body section (docs/prototype/index.html
// 3447–3468) — the per-file retention number with its §6.5 hint and
// the inline error slot. The field renders in its range-error state
// when `error` is non-null: aria-invalid, the visible error line and
// the typed out-of-range value in the input while the stored value
// keeps its place (DEF-001: "the previous value stays until it is
// fixed" — the stored one). No behavior: no application, the number
// is a static control.
import { useId } from 'vue'
import { useI18n } from 'vue-i18n'
import type { SettingNumberRange } from '../../dev/seed'

const props = defineProps<{
  /** The stored retention value (the seed's backup.retention). */
  readonly value: number
  /** The §6.5 closed range (the input's min/max and the error's
   * wording facts). */
  readonly range: SettingNumberRange
  /** The range-error state: the typed out-of-range value the input
   * shows; null is the clean state. */
  readonly error: { readonly value: number } | null
}>()

const { t } = useI18n()

const inputId = useId()
const hintId = useId()
const errorId = useId()
const titleId = useId()
</script>

<template>
  <section class="ps-section" :aria-labelledby="titleId">
    <h3 :id="titleId" class="ps-section-title">{{ t('settings.backupsTitle') }}</h3>

    <div class="ps-field">
      <label class="field-label" :for="inputId">{{ t('settings.backupRetentionLabel') }}</label>
      <input
        :id="inputId"
        class="ps-input set-number"
        type="number"
        :min="props.range.min"
        :max="props.range.max"
        step="1"
        :value="props.error === null ? props.value : props.error.value"
        :aria-invalid="props.error !== null"
        :aria-describedby="`${hintId} ${errorId}`"
      />
      <p :id="hintId" class="ps-hint">{{ t('settings.backupRetentionHint') }}</p>
      <p :id="errorId" class="ps-error" :hidden="props.error === null">
        {{ t('settings.error.backupRetention') }}
      </p>
    </div>
  </section>
</template>
