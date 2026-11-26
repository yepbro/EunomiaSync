<script setup lang="ts">
// The hide-instruction confirmation (T-365, MAIN-012…014, LIB-005,
// UI-010): the mock-up's #hide-dialog (docs/prototype/index.html
// 2748–2769) on the T-363 dialog contract — a dedicated dialog, since
// the shared confirmation is hard-labeled "Delete". No behavior:
// Cancel/Hide are the form's method="dialog" return values; marking a
// row hidden is a behavior card's (MAIN-012's family).
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
      <h2 :id="titleId">{{ t('library.hide.title') }}</h2>
      <p class="modal-text">{{ t('library.hide.text') }}</p>
      <div class="modal-actions">
        <button value="cancel" class="btn btn-ghost">{{ t('library.hide.cancel') }}</button>
        <button value="hide" class="btn btn-primary">{{ t('library.hide.hide') }}</button>
      </div>
    </form>
  </BaseModal>
</template>
