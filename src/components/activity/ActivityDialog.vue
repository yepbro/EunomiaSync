<script setup lang="ts">
// The Activity window (T-367, LOG-001…006, §7.7 UI row): the mock-up's
// #activity-dialog (docs/prototype/index.html 3716–3791) on the T-363
// dialog contract — BaseModal (size lg) + defineModel<boolean>('open').
// The four filters (date presets, project, the five type groups, the
// result) render with their options and the demo's at-rest selections
// ("All …"); the entries render newest-first in prop order (the seed's
// order) with their type chip, the E_* code chip, the ok/skipped/error
// result chip and the file-write path under the project (LOG-004).
// The limits hint interpolates the settings' log limits (LOG-006). No
// behavior: nothing filters — the combining query is the activity
// card's; the selects and the Close button are the mock-up's inert
// controls.
import { useId } from 'vue'
import { useI18n } from 'vue-i18n'
import BaseModal from '../ui/BaseModal.vue'
import type { ActivityEntry, ActivityResult, ActivityType } from '../../dev/seed'
import type { CatalogKey } from '../../i18n'

const props = defineProps<{
  /** The entries, newest first (the seed's demo list). */
  readonly entries: readonly ActivityEntry[]
  /** The project filter's options (the seed's demo projects). */
  readonly projects: readonly string[]
  /** The log limits the hint names (LOG-006; the seed settings' values). */
  readonly logLimits: { readonly entries: number; readonly days: number }
}>()

const { t, n } = useI18n()

/** The dialog contract (T-363): v-model:open drives the BaseModal. */
const open = defineModel<boolean>('open', { default: false })

const titleId = useId()
const dateFilterId = useId()
const projectFilterId = useId()
const typeFilterId = useId()
const resultFilterId = useId()

/** The closed type vocabulary, in the filter's option order (the
 * demo's ACT_TYPES keys). */
const actTypes: readonly ActivityType[] = ['projects', 'library', 'assignments', 'sync', 'backups']

/** The type chip's and filter option's wording key. */
const typeKey = {
  projects: 'activity.type.projects',
  library: 'activity.type.library',
  assignments: 'activity.type.assignments',
  sync: 'activity.type.sync',
  backups: 'activity.type.backups',
} satisfies Record<ActivityType, CatalogKey>

/** The closed result vocabulary (the chips and the filter options). */
const results: readonly ActivityResult[] = ['ok', 'skipped', 'error']

const resultKey = {
  ok: 'activity.result.ok',
  skipped: 'activity.result.skipped',
  error: 'activity.result.error',
} satisfies Record<ActivityResult, CatalogKey>

/** The result chip's tint class per result. */
const resultClass: Record<ActivityResult, string> = {
  ok: 'act-res--ok',
  skipped: 'act-res--skipped',
  error: 'act-res--error',
}

/** The date presets, in the filter's option order (the §7.7 UI row). */
const datePresets: readonly {
  readonly value: string
  readonly key: CatalogKey
}[] = [
  { value: 'all', key: 'activity.dateAll' },
  { value: '7d', key: 'activity.date7d' },
  { value: 'today', key: 'activity.dateToday' },
]
</script>

<template>
  <BaseModal v-model:open="open" size="lg" :labelled-by="titleId">
    <form method="dialog">
      <h2 :id="titleId">{{ t('activity.title') }}</h2>
      <p class="modal-text act-subtitle">{{ t('activity.subtitle') }}</p>

      <div class="act-filters">
        <label class="visually-hidden" :for="dateFilterId">
          {{ t('activity.filterDateLabel') }}
        </label>
        <select :id="dateFilterId">
          <option
            v-for="preset in datePresets"
            :key="preset.value"
            :value="preset.value"
            :selected="preset.value === 'all'"
          >
            {{ t(preset.key) }}
          </option>
        </select>

        <label class="visually-hidden" :for="projectFilterId">
          {{ t('activity.filterProjectLabel') }}
        </label>
        <select :id="projectFilterId">
          <option value="all" selected>{{ t('activity.projectAll') }}</option>
          <option v-for="name in props.projects" :key="name" :value="name">{{ name }}</option>
        </select>

        <label class="visually-hidden" :for="typeFilterId">
          {{ t('activity.filterTypeLabel') }}
        </label>
        <select :id="typeFilterId">
          <option value="all" selected>{{ t('activity.typeAll') }}</option>
          <option v-for="type in actTypes" :key="type" :value="type">
            {{ t(typeKey[type]) }}
          </option>
        </select>

        <label class="visually-hidden" :for="resultFilterId">
          {{ t('activity.filterResultLabel') }}
        </label>
        <select :id="resultFilterId">
          <option value="all" selected>{{ t('activity.resultAll') }}</option>
          <option v-for="result in results" :key="result" :value="result">
            {{ t(resultKey[result]) }}
          </option>
        </select>
      </div>

      <p class="ps-hint act-limits">
        {{
          t('activity.limits', {
            entries: n(props.logLimits.entries),
            days: n(props.logLimits.days),
          })
        }}
      </p>

      <div class="act-body">
        <table class="hist-table act-table">
          <caption class="visually-hidden">
            {{
              t('activity.tableCaption')
            }}
          </caption>
          <thead>
            <tr>
              <th scope="col" class="act-col-when">{{ t('activity.colTime') }}</th>
              <th scope="col">{{ t('activity.colAction') }}</th>
              <th scope="col">{{ t('activity.colObject') }}</th>
              <th scope="col" class="act-col-res">{{ t('activity.colResult') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="entry in props.entries" :key="entry.when">
              <td class="hist-when">{{ entry.when }}</td>
              <td class="act-action">
                <span class="chip act-type">{{ t(typeKey[entry.type]) }}</span>{{ entry.action }}
                <span v-if="entry.code !== null" class="chip act-code">{{ entry.code }}</span>
              </td>
              <td class="act-object">
                {{ entry.project ?? '—' }}
                <code v-if="entry.path !== null" class="act-path">{{ entry.path }}</code>
              </td>
              <td class="hist-op-cell">
                <span class="chip hist-op act-res" :class="resultClass[entry.result]">
                  {{ t(resultKey[entry.result]) }}
                </span>
              </td>
            </tr>
          </tbody>
        </table>
        <p class="hist-empty" :hidden="props.entries.length > 0">
          {{ t('activity.empty') }}
        </p>
      </div>

      <div class="modal-actions">
        <button value="close" class="btn btn-ghost">{{ t('activity.close') }}</button>
      </div>
    </form>
  </BaseModal>
</template>
