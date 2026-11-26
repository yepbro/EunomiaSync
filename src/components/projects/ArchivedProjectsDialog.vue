<script setup lang="ts">
// The archived-projects window (T-366, D-10, PRJ-013…018): the
// mock-up's #archived-dialog (docs/prototype/index.html 3622–3649) on
// the T-363 dialog contract — BaseModal (size lg, the
// `.archived-dialog` class hooking the section's auto height) +
// defineModel<boolean>('open'). One row per archived project in the
// scan-results row idiom: the color dot, the name, the path, the
// "Archived <date>" stamp and the origin wording (the same words the
// settings modal's "Added" row uses). The Restore button repeats the
// project name for screen readers (the demo's aria-label fact,
// app.js ~4207); the restore flow is a behavior card's. The list and
// the empty paragraph both stay in the DOM with the mock-up's hidden
// toggling (UI-012). No behavior: Close is the form's method="dialog"
// return value.
import { computed, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import BaseModal from '../ui/BaseModal.vue'
import type { ArchivedProject } from '../../dev/seed'

const props = defineProps<{
  /** The archived projects (the seed's demo list). */
  readonly rows: readonly ArchivedProject[]
}>()

const { t } = useI18n()

/** The dialog contract (T-363): v-model:open drives the BaseModal. */
const open = defineModel<boolean>('open', { default: false })

const titleId = useId()

const hasRows = computed(() => props.rows.length > 0)

/** The origin wording (the demo's "Manual" / "Found by scan" — the
 * settings modal's "Added" wording). */
const originText = (origin: ArchivedProject['origin']): string =>
  origin === 'manual' ? t('projects.settings.originManual') : t('projects.settings.originScan')
</script>

<template>
  <BaseModal v-model:open="open" size="lg" class="archived-dialog" :labelled-by="titleId">
    <form method="dialog">
      <h2 :id="titleId">{{ t('projects.archived.title') }}</h2>
      <p class="modal-text">{{ t('projects.archived.subtitle') }}</p>

      <div class="ar-body">
        <ul class="ar-list" :hidden="!hasRows">
          <li v-for="row in props.rows" :key="row.name" class="scan-row">
            <span class="proj-color" :style="{ '--c': row.color }" aria-hidden="true"></span>
            <span class="scan-row-name">{{ row.name }}</span>
            <code class="scan-row-path">{{ row.path }}</code>
            <span class="ar-meta">{{
              t('projects.archived.archivedAt', { date: row.archivedAt })
            }}</span>
            <span class="ar-meta">{{ originText(row.origin) }}</span>
            <button
              type="button"
              class="btn btn-ghost ar-restore"
              :aria-label="t('projects.archived.restoreAria', { name: row.name })"
            >
              {{ t('projects.archived.restore') }}
            </button>
          </li>
        </ul>
        <p class="hist-empty" :hidden="hasRows">{{ t('projects.archived.empty') }}</p>
        <p class="ps-hint">{{ t('projects.archived.hint') }}</p>
      </div>

      <div class="modal-actions">
        <button value="close" class="btn btn-ghost">{{ t('projects.archived.close') }}</button>
      </div>
    </form>
  </BaseModal>
</template>
