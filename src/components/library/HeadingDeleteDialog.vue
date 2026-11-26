<script setup lang="ts">
// The delete-section confirmation (T-365, MAIN-015…023, LIB-033,
// §6.4, UI-010): the mock-up's #heading-delete-dialog
// (docs/prototype/index.html 2856–2873) on the T-363 dialog contract.
// The line states the spec's exact fact: only the heading row goes —
// the section's instructions stay in place and become part of the
// previous section. No behavior: Cancel/Delete are the form's
// method="dialog" return values; the deletion is a behavior card's
// (T-185's family).
import { useId } from 'vue'
import { useI18n } from 'vue-i18n'
import BaseModal from '../ui/BaseModal.vue'

const { t } = useI18n()

/** The dialog contract (T-363): v-model:open drives the BaseModal. */
const open = defineModel<boolean>('open', { default: false })

const titleId = useId()
</script>

<template>
  <BaseModal v-model:open="open" :labelled-by="titleId">
    <form method="dialog">
      <h2 :id="titleId">{{ t('library.headingDelete.title') }}</h2>
      <p class="modal-text">{{ t('library.headingDelete.text') }}</p>
      <div class="modal-actions">
        <button value="cancel" class="btn btn-ghost">
          {{ t('library.headingDelete.cancel') }}
        </button>
        <button value="delete" class="btn btn-danger">
          {{ t('library.headingDelete.delete') }}
        </button>
      </div>
    </form>
  </BaseModal>
</template>
