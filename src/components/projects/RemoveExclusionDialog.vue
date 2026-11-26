<script setup lang="ts">
// The remove-excluded-path confirmation (T-366, D-10, §6.4, UI-010):
// the mock-up's #remove-exclusion-dialog (docs/prototype/index.html
// 3696–3715) on the T-363 dialog contract — a dedicated dialog
// because the shared confirmation is hard-labeled "Delete". The line
// states the spec's facts: the path is found by the scan again and
// its projects can be added as usual (the demo's composition,
// app.js ~4336). With no prop the mock-up's static "Placeholder."
// stands in (the BulkConfirmDialog precedent). No behavior:
// Cancel/Remove are the form's method="dialog" return values; the
// removal flow is a behavior card's.
import { computed, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import BaseModal from '../ui/BaseModal.vue'

const props = withDefaults(
  defineProps<{
    /** The excluded path the dialog names (absent: the mock-up's
     * "Placeholder." static state). */
    readonly path?: string
  }>(),
  { path: undefined },
)

const { t } = useI18n()

/** The dialog contract (T-363): v-model:open drives the BaseModal. */
const open = defineModel<boolean>('open', { default: false })

const titleId = useId()

const line = computed(() =>
  props.path === undefined
    ? t('projects.removeExclusion.placeholder')
    : t('projects.removeExclusion.text', { path: props.path }),
)
</script>

<template>
  <BaseModal v-model:open="open" :labelled-by="titleId">
    <form method="dialog">
      <h2 :id="titleId">{{ t('projects.removeExclusion.title') }}</h2>
      <p class="modal-text">{{ line }}</p>
      <div class="modal-actions">
        <button value="cancel" class="btn btn-ghost">
          {{ t('projects.removeExclusion.cancel') }}
        </button>
        <button value="remove" class="btn btn-primary">
          {{ t('projects.removeExclusion.confirm') }}
        </button>
      </div>
    </form>
  </BaseModal>
</template>
