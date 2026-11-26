<script setup lang="ts">
// The archive confirmation (T-366, PRJ-013…018, UI-010, NFR-021): the
// mock-up's #archive-dialog (docs/prototype/index.html 3140–3162) on
// the T-363 dialog contract. The explanation states the spec's facts
// — the folder is not touched, assignments are kept, the project
// leaves the syncs; a scan-found project's path goes to the
// exclusions (PRJ-017); the way back through "Archived projects"
// (PRJ-013…018) — composed from catalog keys like the demo composes
// it (app.js ~3280). With no props the mock-up's static
// "Placeholder." stands in (the BulkConfirmDialog precedent), so the
// registered gallery entry shows the reference's static state. The
// confirm button is disabled at rest: matching the typed name is
// T-165's behavior (the field is inert here).
import { computed, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import BaseModal from '../ui/BaseModal.vue'
import type { ProjectOrigin } from '../../dev/seed'

const props = withDefaults(
  defineProps<{
    /** The project the dialog names (absent: the mock-up's
     * "Placeholder." static state). */
    readonly projectName?: string
    /** The project's origin — a scan-found project's line gains the
     * exclusions clause (PRJ-017). */
    readonly origin?: ProjectOrigin
  }>(),
  { projectName: undefined, origin: 'manual' },
)

const { t } = useI18n()

/** The dialog contract (T-363): v-model:open drives the BaseModal. */
const open = defineModel<boolean>('open', { default: false })

const titleId = useId()
const inputId = useId()

/** The explanation line: intro + the scan clause + the way back. */
const line = computed(() => {
  if (props.projectName === undefined) return t('projects.archive.placeholder')
  const intro = t('projects.archive.intro', { name: props.projectName })
  const scanClause = props.origin === 'scan' ? t('projects.archive.scanSuffix') : ''
  return intro + scanClause + t('projects.archive.wayBack')
})
</script>

<template>
  <BaseModal v-model:open="open" :labelled-by="titleId">
    <form method="dialog">
      <h2 :id="titleId">{{ t('projects.archive.title') }}</h2>
      <p class="modal-text">{{ line }}</p>
      <div class="ps-field">
        <label class="field-label" :for="inputId">{{ t('projects.archive.inputLabel') }}</label>
        <input :id="inputId" class="ps-input" type="text" autocomplete="off" />
      </div>
      <div class="modal-actions">
        <button value="cancel" class="btn btn-ghost">{{ t('projects.archive.cancel') }}</button>
        <button value="archive" class="btn btn-danger" disabled>
          {{ t('projects.archive.confirm') }}
        </button>
      </div>
    </form>
  </BaseModal>
</template>
