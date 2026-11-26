<script setup lang="ts">
// The unsaved-changes guard (T-365, UI-010): the mock-up's
// #unsaved-dialog (docs/prototype/index.html 2719–2732) on the T-363
// dialog contract — opened from the editor's Cancel/Esc when something
// changed. No behavior: the guard never fires here (change tracking is
// a behavior card's, T-189's family); Keep editing/Discard are the
// form's method="dialog" return values.
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
      <h2 :id="titleId">{{ t('library.unsaved.title') }}</h2>
      <p class="modal-text">{{ t('library.unsaved.text') }}</p>
      <div class="modal-actions">
        <button value="keep" class="btn btn-ghost">{{ t('library.unsaved.keep') }}</button>
        <button value="discard" class="btn btn-danger">{{ t('library.unsaved.discard') }}</button>
      </div>
    </form>
  </BaseModal>
</template>
