<script setup lang="ts">
// The bulk-assign dialog (T-364, §7.4 bulk operations, UI-002): the
// mock-up's #bulk-dialog (docs/prototype/index.html 2770–2790) on the
// T-363 dialog contract — BaseModal + defineModel<boolean>('open').
// The project multi-select rows (label.ps-check-row.bulk-row with the
// color dot) and the single-row variant radio block are the mock-up's
// markup facts (the rows app.js builds in openBulkDialog, app.js
// 979–1019). All data arrives through props: `projects` (the column
// order) and `variants` (shown only when exactly one instruction row
// is selected — undefined in the skeleton hides the block). No
// behavior: the Apply/Cancel buttons are the dialog's own
// method="dialog" return values, and the real assignment flow is the
// bulk-operations card's (T-184's family). The .ps-field/.ps-check-row
// styles live in src/styles/form.css (the shared form primitives).
import { computed, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import BaseModal from '../ui/BaseModal.vue'
import type { Project } from '../../dev/seed'

/** One variant choice of the single-instruction block. */
export type BulkVariantChoice = {
  /** The option's display name. */
  readonly name: string
  /** The main variant — the demo labels it "Main (main variant)". */
  readonly isMain?: boolean
}

const open = defineModel<boolean>('open', { default: false })

const props = withDefaults(
  defineProps<{
    /** The projects to multi-select, the column order. */
    readonly projects?: readonly Project[]
    /** The variant choices; rendered only for one selected row. */
    readonly variants?: readonly BulkVariantChoice[]
  }>(),
  { projects: () => [], variants: undefined },
)

const { t } = useI18n()

const titleId = useId()
const variantGroupName = useId()

const hasVariants = computed(() => (props.variants?.length ?? 0) > 0)

const optionLabel = (choice: BulkVariantChoice): string =>
  choice.isMain ? t('matrix.bulk.mainVariantLabel') : choice.name
</script>

<template>
  <BaseModal v-model:open="open" :labelled-by="titleId">
    <form method="dialog" novalidate>
      <h2 :id="titleId">{{ t('matrix.bulk.title') }}</h2>
      <p class="modal-text">{{ t('matrix.bulk.text') }}</p>

      <fieldset class="ps-field">
        <legend class="field-label">{{ t('matrix.bulk.projectsLegend') }}</legend>
        <div class="bulk-projects">
          <label v-for="project in projects" :key="project.name" class="ps-check-row bulk-row">
            <input type="checkbox" :value="project.name" />
            <span class="proj-color" :style="{ '--c': project.color }" aria-hidden="true"></span>
            <span>{{ project.name }}</span>
          </label>
        </div>
      </fieldset>

      <div v-if="hasVariants" class="ps-field">
        <span class="field-label">{{ t('matrix.bulk.variantLabel') }}</span>
        <div class="bulk-variants">
          <label
            v-for="(choice, index) in variants"
            :key="choice.name"
            class="ps-check-row bulk-row"
          >
            <input
              type="radio"
              :name="variantGroupName"
              :value="choice.name"
              :checked="index === 0"
            />
            <span>{{ optionLabel(choice) }}</span>
          </label>
        </div>
      </div>

      <div class="modal-actions">
        <button value="cancel" class="btn btn-ghost">{{ t('matrix.bulk.cancel') }}</button>
        <button value="apply" class="btn btn-primary">{{ t('matrix.bulk.apply') }}</button>
      </div>
    </form>
  </BaseModal>
</template>
