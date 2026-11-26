<script setup lang="ts">
// The settings Backups panel (T-366, BKP-004, BKP-012): the mock-up's
// #ps-panel-backups — the copies table (date, file, size, sync run
// number) ending in the narrow "Save to a chosen location" action
// column with a visually-hidden header label; the icon button's
// tooltip names the file and the backup date (BKP-012's tooltip
// fact). The rows arrive through props (the seed carries the demo
// builder's output — src/dev/seed/backups.ts). The save flow (the OS
// save dialog, the verify) is a behavior card's; the button is inert.
import { useI18n } from 'vue-i18n'
import type { BackupRow } from '../../dev/seed'

defineProps<{
  /** The copies of the shown project (BKP-004: the demo builder's
   * output as display strings). */
  readonly backups: readonly BackupRow[]
}>()

const { t } = useI18n()

/** The save action's tooltip/accessible name: the file and the
 * backup date (BKP-012; the demo's saveTitle, app.js ~3057). */
const saveTitle = (row: BackupRow): string =>
  t('projects.settings.saveTitle', { file: row.file, date: row.date })
</script>

<template>
  <table class="ps-backups">
    <thead>
      <tr>
        <th scope="col">{{ t('projects.settings.colWhen') }}</th>
        <th scope="col">{{ t('projects.settings.colFile') }}</th>
        <th scope="col">{{ t('projects.settings.colSize') }}</th>
        <th scope="col">{{ t('projects.settings.colRun') }}</th>
        <th scope="col" class="ps-backup-save">
          <span class="visually-hidden">{{ t('projects.settings.colSave') }}</span>
        </th>
      </tr>
    </thead>
    <tbody>
      <tr v-for="row in backups" :key="row.when">
        <td>{{ row.when }}</td>
        <td class="ps-backup-file">{{ row.file }}</td>
        <td>{{ row.size }}</td>
        <td>{{ row.run }}</td>
        <td class="ps-backup-save">
          <button
            type="button"
            class="cell-icon cell-icon--save"
            :title="saveTitle(row)"
            :aria-label="saveTitle(row)"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M16 13h-3V3h-2v10H8l4 4 4-4zM4 19v2h16v-2H4z" />
            </svg>
          </button>
        </td>
      </tr>
    </tbody>
  </table>
  <p class="ps-hint">{{ t('projects.settings.backupsHint') }}</p>
</template>
