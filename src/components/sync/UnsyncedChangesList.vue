<script setup lang="ts">
// The "Unsynced changes" list (T-367, MAIN-026): the mock-up's
// #ss-pending-list block of #sync-summary-dialog — one row per dirty
// project with the project and its target file (scheme C writes
// `<STEM>.project.md`, §9.4), rendered as the reference's two-column
// table, plus the "Everything is synced." empty state (the demo's
// hidden toggling: the list and the empty paragraph swap, UI-12's
// idiom). The sync card's own list component (the T-367 structure):
// SyncSummaryDialog renders it in the pending view. No behavior: the
// rows are props, nothing filters or refreshes them.
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import type { SyncPendingRow } from '../../dev/seed'

const props = defineProps<{
  /** The dirty projects with their target files (the seed's demo list). */
  readonly rows: readonly SyncPendingRow[]
}>()

const { t } = useI18n()

const hasRows = computed(() => props.rows.length > 0)
</script>

<template>
  <div class="ss-body" :hidden="!hasRows">
    <table class="ss-table">
      <caption class="visually-hidden">
        {{
          t('sync.pendingCaption')
        }}
      </caption>
      <thead>
        <tr>
          <th scope="col" class="ss-col-proj">{{ t('sync.colProject') }}</th>
          <th scope="col">{{ t('sync.colTargetFile') }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="row in props.rows" :key="row.project">
          <td class="ss-proj">{{ row.project }}</td>
          <td class="ss-file">
            <code>{{ row.targetFile }}</code>
          </td>
        </tr>
      </tbody>
    </table>
  </div>
  <p class="hist-empty" :hidden="hasRows">{{ t('sync.pendingEmpty') }}</p>
</template>
