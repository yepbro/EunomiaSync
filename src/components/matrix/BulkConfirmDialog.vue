<script setup lang="ts">
// The bulk-change confirmation (T-364, UI-010, NFR-021): the mock-up's
// #bulk-confirm-dialog (docs/prototype/index.html 2792–2804) on the
// T-363 dialog contract — BaseModal + defineModel<boolean>('open').
// Shared by the bulk dialog and the column menu: it states the exact
// number of changed assignments before anything is applied. The text
// arrives through the `text` prop (the caller composes the count);
// with no prop the mock-up's "Placeholder." stands in — the demo's
// static fact (its JS fills the line at runtime, app.js ~1136).
// No behavior: Continue/Cancel are the dialog's own method="dialog"
// return values; the real flow is the bulk-operations card's.
import { computed, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import BaseModal from '../ui/BaseModal.vue'

const open = defineModel<boolean>('open', { default: false })

const props = withDefaults(
  defineProps<{
    /** The confirmation line (the caller names the change count). */
    readonly text?: string
  }>(),
  { text: undefined },
)

const { t } = useI18n()

const titleId = useId()

const line = computed(() => props.text ?? t('matrix.bulkConfirm.placeholder'))
</script>

<template>
  <BaseModal v-model:open="open" :labelled-by="titleId">
    <form method="dialog">
      <h2 :id="titleId">{{ t('matrix.bulkConfirm.title') }}</h2>
      <p class="modal-text">{{ line }}</p>
      <div class="modal-actions">
        <button value="cancel" class="btn btn-ghost">{{ t('matrix.bulkConfirm.cancel') }}</button>
        <button value="continue" class="btn btn-primary">
          {{ t('matrix.bulkConfirm.continue') }}
        </button>
      </div>
    </form>
  </BaseModal>
</template>
