<script setup lang="ts">
// The restore-variant confirmation (T-365, LIB-027): the mock-up's
// #restore-dialog (docs/prototype/index.html 3200–3220) on the T-363
// dialog contract. The variant's name arrives through the prop (the
// demo fills the line from the delete row's snapshot, app.js ~2532).
// No behavior: Cancel/Restore are the form's method="dialog" return
// values; the real restore is a behavior card's.
import { useId } from 'vue'
import { useI18n } from 'vue-i18n'
import BaseModal from '../ui/BaseModal.vue'

defineProps<{
  /** The variant the confirmation names. */
  readonly variantName: string
}>()

const { t } = useI18n()

/** The dialog contract (T-363): v-model:open drives the BaseModal. */
const open = defineModel<boolean>('open', { default: false })

const titleId = useId()
</script>

<template>
  <BaseModal v-model:open="open" :labelled-by="titleId">
    <form method="dialog">
      <h2 :id="titleId">{{ t('library.restore.title') }}</h2>
      <p class="modal-text">{{ t('library.restore.text', { name: variantName }) }}</p>
      <div class="modal-actions">
        <button value="cancel" class="btn btn-ghost">{{ t('library.restore.cancel') }}</button>
        <button value="restore" class="btn btn-primary">{{ t('library.restore.restore') }}</button>
      </div>
    </form>
  </BaseModal>
</template>
