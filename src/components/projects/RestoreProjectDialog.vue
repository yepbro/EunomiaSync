<script setup lang="ts">
// The restore-archived-project confirmation (T-366, PRJ-018, D-10,
// UI-010): the mock-up's #restore-project-dialog (docs/prototype/
// index.html 3650–3668) on the T-363 dialog contract — a dedicated
// dialog because the shared confirmation is hard-labeled "Delete".
// The line states the spec's facts: the project comes back active
// with its assignments and its path leaves the exclusions so a scan
// finds it again (the demo's composition, app.js ~4242). With no prop
// the mock-up's static "Placeholder." stands in (the BulkConfirmDialog
// precedent). No behavior: Cancel/Restore are the form's
// method="dialog" return values; the restore flow is a behavior
// card's.
import { computed, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import BaseModal from '../ui/BaseModal.vue'

const props = withDefaults(
  defineProps<{
    /** The project the dialog names (absent: the mock-up's
     * "Placeholder." static state). */
    readonly projectName?: string
  }>(),
  { projectName: undefined },
)

const { t } = useI18n()

/** The dialog contract (T-363): v-model:open drives the BaseModal. */
const open = defineModel<boolean>('open', { default: false })

const titleId = useId()

const line = computed(() =>
  props.projectName === undefined
    ? t('projects.restore.placeholder')
    : t('projects.restore.text', { name: props.projectName }),
)
</script>

<template>
  <BaseModal v-model:open="open" :labelled-by="titleId">
    <form method="dialog">
      <h2 :id="titleId">{{ t('projects.restore.title') }}</h2>
      <p class="modal-text">{{ line }}</p>
      <div class="modal-actions">
        <button value="cancel" class="btn btn-ghost">{{ t('projects.restore.cancel') }}</button>
        <button value="restore" class="btn btn-primary">
          {{ t('projects.restore.confirm') }}
        </button>
      </div>
    </form>
  </BaseModal>
</template>
