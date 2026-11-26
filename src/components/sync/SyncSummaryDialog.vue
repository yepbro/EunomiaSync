<script setup lang="ts">
// The sync summary dialog (T-367, MAIN-024…031, OUT-009): the
// mock-up's #sync-summary-dialog (docs/prototype/index.html 3283–3367)
// on the T-363 dialog contract — BaseModal (size lg, the
// `.sync-summary-dialog` class hooking the footer's hidden-button
// rule) + defineModel<boolean>('open'). The reference's one dialog
// carries TWO views, toggled by the demo's JS through the `hidden`
// attributes of #ss-result/#ss-pending; here the view is a prop
// ('summary' | 'pending') — showing one over the other is the sync
// card's behavior, both render at rest from the gallery. Everything
// visible is derived from props: the outcome badge and the four
// counter chips are computed from the rows (runOutcomeOf's rule — any
// skip or failure makes the run partial), one row per project with
// path, reason and the E_* code + what-to-do hint of the
// skipped/failed rows (UI-011). The status words, counter labels and
// outcome note are the catalog's (closed vocabularies); the reasons,
// codes and hints are seed data. No behavior: no run, no close-driven
// run start — the "Run sync" button is the mock-up's inert control,
// hidden in the summary view and disabled on an empty pending list
// (the demo's renderPendingList facts).
import { computed, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import BaseModal from '../ui/BaseModal.vue'
import UnsyncedChangesList from './UnsyncedChangesList.vue'
import type { SyncPendingRow, SyncStatus, SyncSummaryRow } from '../../dev/seed'

const props = defineProps<{
  /** Which view renders: the run summary or the pending list. */
  readonly view: 'summary' | 'pending'
  /** The run's per-project rows (the summary view's data). */
  readonly rows: readonly SyncSummaryRow[]
  /** The dirty projects with their target files (the pending view). */
  readonly pendingRows: readonly SyncPendingRow[]
}>()

const { t } = useI18n()

/** The dialog contract (T-363): v-model:open drives the BaseModal. */
const open = defineModel<boolean>('open', { default: false })

const titleId = useId()

/** The rows carrying a code + hint (the union's coded arm). */
type SyncCodedRow = Extract<SyncSummaryRow, { code: string }>

const hasCode = (row: SyncSummaryRow): row is SyncCodedRow =>
  row.status === 'skipped' || row.status === 'failed'

/** The four counter chips' values (the demo's n() helper as computeds). */
const counts = computed(() => {
  const n = (status: SyncStatus): number => props.rows.filter((row) => row.status === status).length
  return {
    written: n('written'),
    unchanged: n('unchanged'),
    skipped: n('skipped'),
    failed: n('failed'),
  }
})

/** The run's final status (§9.4 phase 4, the demo's runOutcomeOf):
 * success when every project was written or left unchanged, partial
 * otherwise. */
const outcome = computed(() =>
  props.rows.some((row) => row.status === 'skipped' || row.status === 'failed')
    ? 'partial'
    : 'success',
)

/** The outcome badge's wording key (the closed §6.2 sync_runs enum). */
const outcomeKey: Record<'success' | 'partial', string> = {
  success: 'sync.outcome.success',
  partial: 'sync.outcome.partial',
}

/** The outcome badge's tint class. */
const outcomeClass: Record<'success' | 'partial', string> = {
  success: 'ss-run-badge--success',
  partial: 'ss-run-badge--partial',
}

/** The outcome's short explanation (the demo's ssRunNote copy). */
const outcomeNoteKey: Record<'success' | 'partial', string> = {
  success: 'sync.outcomeNote.success',
  partial: 'sync.outcomeNote.partial',
}

/** The status badge's wording key (the closed §6.2 status enum, not
 * seed data). */
const statusKey: Record<SyncStatus, string> = {
  written: 'sync.status.written',
  unchanged: 'sync.status.unchanged',
  skipped: 'sync.status.skipped',
  failed: 'sync.status.failed',
}

/** The status badge's tint class per status. */
const statusClass: Record<SyncStatus, string> = {
  written: 'ss-st--written',
  unchanged: 'ss-st--unchanged',
  skipped: 'ss-st--skipped',
  failed: 'ss-st--failed',
}
</script>

<template>
  <BaseModal v-model:open="open" size="lg" class="sync-summary-dialog" :labelled-by="titleId">
    <form method="dialog">
      <h2 v-if="props.view === 'summary'" :id="titleId">{{ t('sync.summaryTitle') }}</h2>
      <h2 v-else :id="titleId">{{ t('sync.pendingTitle') }}</h2>

      <!-- Result view: outcome + per-project table -->
      <div v-if="props.view === 'summary'" class="ss-result">
        <p class="modal-text ss-subtitle">
          <span class="chip ss-run-badge" :class="outcomeClass[outcome]">
            {{ t(outcomeKey[outcome]) }}
          </span>
          <span>{{ t(outcomeNoteKey[outcome]) }}</span>
        </p>

        <div class="ss-counters" role="group" :aria-label="t('sync.countersLabel')">
          <span class="chip ss-count ss-count--written">
            {{ t('sync.countWritten') }}
            <strong>{{ counts.written }}</strong>
          </span>
          <span class="chip ss-count ss-count--unchanged">
            {{ t('sync.countUnchanged') }}
            <strong>{{ counts.unchanged }}</strong>
          </span>
          <span class="chip ss-count ss-count--skipped">
            {{ t('sync.countSkipped') }}
            <strong>{{ counts.skipped }}</strong>
          </span>
          <span class="chip ss-count ss-count--failed">
            {{ t('sync.countFailed') }}
            <strong>{{ counts.failed }}</strong>
          </span>
        </div>

        <div class="ss-body">
          <table class="ss-table">
            <caption class="visually-hidden">
              {{
                t('sync.summaryCaption')
              }}
            </caption>
            <thead>
              <tr>
                <th scope="col" class="ss-col-proj">{{ t('sync.colProject') }}</th>
                <th scope="col">{{ t('sync.colTargetFile') }}</th>
                <th scope="col" class="ss-col-status">{{ t('sync.colStatus') }}</th>
                <th scope="col">{{ t('sync.colReason') }}</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="row in props.rows" :key="row.project">
                <td class="ss-proj">{{ row.project }}</td>
                <td class="ss-file">
                  <code>{{ row.targetFile }}</code>
                </td>
                <td class="ss-st-cell">
                  <span class="chip ss-st" :class="statusClass[row.status]">
                    {{ t(statusKey[row.status]) }}
                  </span>
                </td>
                <td class="ss-reason">
                  <span class="ss-reason-text">{{ row.reason }}</span>
                  <!-- Skip and failure rows carry their E_* code and the
                       what-to-do hint (UI-011). -->
                  <span v-if="hasCode(row)" class="ss-code-row">
                    <span class="chip ss-code">{{ row.code }}</span>
                    <span class="ss-hint">{{ row.hint }}</span>
                  </span>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>

      <!-- Pending view: projects with unsynced changes (MAIN-026) -->
      <div v-else class="ss-pending">
        <p class="modal-text">{{ t('sync.pendingSubtitle') }}</p>
        <UnsyncedChangesList :rows="props.pendingRows" />
        <p class="ps-hint">{{ t('sync.pendingHint') }}</p>
      </div>

      <div class="modal-actions">
        <button value="close" class="btn btn-ghost">{{ t('sync.close') }}</button>
        <!-- The pending view's convenience start (MAIN-026); hidden in
             the summary view, disabled on an empty list — the demo's
             renderPendingList facts. Starting the run is a behavior
             card's. -->
        <button
          type="button"
          class="btn btn-primary"
          :hidden="props.view === 'summary'"
          :disabled="props.pendingRows.length === 0"
        >
          {{ t('sync.runSync') }}
        </button>
      </div>
    </form>
  </BaseModal>
</template>
