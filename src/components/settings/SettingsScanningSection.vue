<script setup lang="ts">
// The Settings window's "Scanning" section (T-367, DEF-001…004, D-05):
// the mock-up's third set-body section (docs/prototype/index.html
// 3470–3515) — the maximum folder depth number (with its range-error
// state, like the Backups field) and the excluded folder names as
// chips: one chip row per name with an accessible remove cross, the
// add row (a labelled input + the Add button) and the inline error
// slot (hidden at rest — the add validation is the settings card's).
// No behavior: nothing filters, adds or removes.
import { useId } from 'vue'
import { useI18n } from 'vue-i18n'
import type { SettingNumberRange } from '../../dev/seed'

const props = defineProps<{
  /** The stored depth value (the seed's scan.maxDepth). */
  readonly depth: number
  /** The §6.5 closed range. */
  readonly range: SettingNumberRange
  /** The range-error state (the typed out-of-range value); null is
   * the clean state. */
  readonly error: { readonly value: number } | null
  /** The excluded folder names (the seed's scan.excludeNames). */
  readonly excludeNames: readonly string[]
}>()

const { t } = useI18n()

const depthFieldId = useId()
const depthHintId = useId()
const depthErrorId = useId()
const exclLabelId = useId()
const exclInputId = useId()
const exclErrorId = useId()
const titleId = useId()
</script>

<template>
  <section class="ps-section" :aria-labelledby="titleId">
    <h3 :id="titleId" class="ps-section-title">{{ t('settings.scanningTitle') }}</h3>

    <div class="ps-field">
      <label class="field-label" :for="depthFieldId">{{ t('settings.scanDepthLabel') }}</label>
      <input
        :id="depthFieldId"
        class="ps-input set-number"
        type="number"
        :min="props.range.min"
        :max="props.range.max"
        step="1"
        :value="props.error === null ? props.depth : props.error.value"
        :aria-invalid="props.error !== null"
        :aria-describedby="`${depthHintId} ${depthErrorId}`"
      />
      <p :id="depthHintId" class="ps-hint">{{ t('settings.scanDepthHint') }}</p>
      <p :id="depthErrorId" class="ps-error" :hidden="props.error === null">
        {{ t('settings.error.scanDepth') }}
      </p>
    </div>

    <div class="ps-field">
      <span :id="exclLabelId" class="field-label">{{ t('settings.exclusionsLabel') }}</span>
      <div class="set-excl-list" role="group" :aria-labelledby="exclLabelId">
        <div v-for="name in props.excludeNames" :key="name" class="set-excl-row">
          <span class="chip">{{ name }}</span>
          <button
            type="button"
            class="cell-icon cell-icon--delete set-excl-remove"
            :title="t('settings.exclusionRemoveTitle', { name })"
            :aria-label="t('settings.exclusionRemoveAria', { name })"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path
                d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z"
              />
            </svg>
          </button>
        </div>
      </div>
      <div class="set-excl-add">
        <label class="visually-hidden" :for="exclInputId">
          {{ t('settings.exclusionsInputLabel') }}
        </label>
        <input
          :id="exclInputId"
          class="ps-input"
          type="text"
          autocomplete="off"
          maxlength="100"
          :placeholder="t('settings.exclusionsPlaceholder')"
          :aria-describedby="exclErrorId"
        />
        <button type="button" class="btn btn-ghost">{{ t('settings.exclusionsAdd') }}</button>
      </div>
      <p :id="exclErrorId" class="ps-error" hidden></p>
      <p class="ps-hint">{{ t('settings.exclusionsHint') }}</p>
    </div>
  </section>
</template>
