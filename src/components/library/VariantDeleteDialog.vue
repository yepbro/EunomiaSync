<script setup lang="ts">
// The variant-deletion confirmation (T-365, LIB-023, UI-010): the
// mock-up's #confirm-dialog ported for this use (the shared
// confirmation, docs/prototype/index.html 2733–2747; the demo fills
// its title and line per deleted variant, app.js requestDeleteVariant
// ~1889). The spec wins over the demo's static line: LIB-023 names the
// NUMBER of affected projects, so the count arrives as a prop. The
// question and the count phrase are two catalog keys composed here —
// vue-i18n pipe plurals are whole-message alternatives, so one piped
// message would lose the question in the 1/other forms (round-1
// review). No behavior: Cancel/Delete are the form's method="dialog"
// return values; the deletion and the assignment move are a behavior
// card's (T-191's family).
import { computed, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import BaseModal from '../ui/BaseModal.vue'

const props = defineProps<{
  /** The variant the confirmation names. */
  readonly variantName: string
  /** The number of projects whose assignments move to the main
   * variant (LIB-023). */
  readonly affectedProjects: number
}>()

const { t, n } = useI18n()

/** The dialog contract (T-363): v-model:open drives the BaseModal. */
const open = defineModel<boolean>('open', { default: false })

const titleId = useId()

const line = computed(
  () =>
    `${t('library.variantDelete.question', { name: props.variantName })} ${t('library.variantDelete.count', { count: n(props.affectedProjects) }, props.affectedProjects)}`,
)
</script>

<template>
  <BaseModal v-model:open="open" :labelled-by="titleId">
    <form method="dialog">
      <h2 :id="titleId">{{ t('library.variantDelete.title') }}</h2>
      <p class="modal-text">{{ line }}</p>
      <div class="modal-actions">
        <button value="cancel" class="btn btn-ghost">
          {{ t('library.variantDelete.cancel') }}
        </button>
        <button value="delete" class="btn btn-danger">
          {{ t('library.variantDelete.delete') }}
        </button>
      </div>
    </form>
  </BaseModal>
</template>
