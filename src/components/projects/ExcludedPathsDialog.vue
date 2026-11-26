<script setup lang="ts">
// The excluded-paths window (T-366, D-10, §6.2 project_exclusions):
// the mock-up's #exclusions-dialog (docs/prototype/index.html
// 3669–3695) on the T-363 dialog contract — BaseModal (size lg, the
// `.excluded-dialog` class hooking the section's auto height) +
// defineModel<boolean>('open'). One row per excluded path in the
// archived row idiom: the path, a human-readable reason badge
// ("Removed by archiving" / "Added manually" — the wording carries
// the meaning, the tint only supports it) and the "Added <date>"
// stamp. The Remove button repeats the path for screen readers (the
// demo's aria-label fact, app.js ~4302); the removal flow is a
// behavior card's. The list and the empty paragraph both stay in the
// DOM with the mock-up's hidden toggling (UI-012). No behavior:
// Close is the form's method="dialog" return value.
import { computed, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import BaseModal from '../ui/BaseModal.vue'
import type { ExcludedPath, ExclusionReason } from '../../dev/seed'
import type { CatalogKey } from '../../i18n'

const props = defineProps<{
  /** The excluded paths (the seed's demo list). */
  readonly rows: readonly ExcludedPath[]
}>()

const { t } = useI18n()

/** The dialog contract (T-363): v-model:open drives the BaseModal. */
const open = defineModel<boolean>('open', { default: false })

const titleId = useId()

const hasRows = computed(() => props.rows.length > 0)

/** The reason badge's wording key (the demo's EXCLUSION_REASONS table
 * is catalog copy keyed by the DDL enum). */
const reasonKey = {
  removed_scanned: 'projects.excluded.reasonRemoved',
  manual: 'projects.excluded.reasonManual',
} satisfies Record<ExclusionReason, CatalogKey>

/** The badge's modifier class (a class name, not copy — computed in
 * script; the template-only raw-text rule reads template literals). */
const reasonClass = (row: ExcludedPath): string => `ex-reason--${row.reason}`
</script>

<template>
  <BaseModal v-model:open="open" size="lg" class="excluded-dialog" :labelled-by="titleId">
    <form method="dialog">
      <h2 :id="titleId">{{ t('projects.excluded.title') }}</h2>
      <p class="modal-text">{{ t('projects.excluded.subtitle') }}</p>

      <div class="ar-body">
        <ul class="ar-list" :hidden="!hasRows">
          <li v-for="row in props.rows" :key="row.path" class="scan-row">
            <code class="scan-row-path">{{ row.path }}</code>
            <span class="chip ex-reason" :class="reasonClass(row)">
              {{ t(reasonKey[row.reason]) }}
            </span>
            <span class="ar-meta">{{ t('projects.excluded.addedAt', { date: row.addedAt }) }}</span>
            <button
              type="button"
              class="btn btn-ghost ex-remove"
              :aria-label="t('projects.excluded.removeAria', { path: row.path })"
            >
              {{ t('projects.excluded.remove') }}
            </button>
          </li>
        </ul>
        <p class="hist-empty" :hidden="hasRows">{{ t('projects.excluded.empty') }}</p>
        <p class="ps-hint">{{ t('projects.excluded.hint') }}</p>
      </div>

      <div class="modal-actions">
        <button value="close" class="btn btn-ghost">{{ t('projects.excluded.close') }}</button>
      </div>
    </form>
  </BaseModal>
</template>
