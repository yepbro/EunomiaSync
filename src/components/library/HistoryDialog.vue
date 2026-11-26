<script setup lang="ts">
// The instruction history panel (T-365, LIB-026/027, D-16): the
// mock-up's #history-dialog (docs/prototype/index.html 3163–3199) on
// the T-363 dialog contract, size lg. The revisions table lists the
// versions (number, time, operation, entity and snapshot disclosure)
// from the `revisions` prop — the seed carries the demo builder's
// output (src/dev/seed/history.ts). Data-derived rendering: the op
// chip class follows the revision's op, a delete row carries the
// Restore variant button (LIB-027 — inert; the restore flow is a
// behavior card's), a restored delete row shows the "Restored" chip
// (the seed starts with none), the snapshot body follows the
// snapshot's kind (an instruction version stores name + state, a
// variant version name + text, D-16), and the empty state shows when
// no revisions arrive. The table and the empty paragraph both stay in
// the DOM with the mock-up's hidden toggling (app.js renderRevisions).
// No behavior: Close is the form's method="dialog" return value.
import { computed, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import BaseModal from '../ui/BaseModal.vue'
import type { HistoryRevision } from '../../dev/seed'

const props = defineProps<{
  /** The instruction the panel titles itself with (the demo's
   * "History — <name>"; an empty name falls back to the demo's
   * literal "instruction"). */
  readonly instructionName: string
  /** The revisions, newest first (the seed's demo data). */
  readonly revisions: readonly HistoryRevision[]
}>()

const { t } = useI18n()

/** The dialog contract (T-363): v-model:open drives the BaseModal. */
const open = defineModel<boolean>('open', { default: false })

const titleId = useId()

const hasRevisions = computed(() => props.revisions.length > 0)

const rowKey = (rev: HistoryRevision): string => `${rev.entityType}-${rev.entityName}-${rev.revNo}`

/** The op chip's modifier class (a class name, not copy — computed in
 * script; the template-only raw-text rule reads template literals). */
const opClass = (rev: HistoryRevision): string => `hist-op--${rev.op}`
</script>

<template>
  <BaseModal v-model:open="open" size="lg" :labelled-by="titleId">
    <form method="dialog">
      <h2 :id="titleId">
        {{
          instructionName !== ''
            ? t('library.history.title', { name: instructionName })
            : t('library.history.titleFallback')
        }}
      </h2>
      <p class="modal-text hist-subtitle">{{ t('library.history.subtitle') }}</p>

      <div class="hist-body">
        <table class="hist-table" :hidden="!hasRevisions">
          <caption class="visually-hidden">
            {{
              t('library.history.caption')
            }}
          </caption>
          <thead>
            <tr>
              <th scope="col" class="hist-col-no">{{ t('library.history.colNo') }}</th>
              <th scope="col" class="hist-col-when">{{ t('library.history.colWhen') }}</th>
              <th scope="col" class="hist-col-op">{{ t('library.history.colOp') }}</th>
              <th scope="col" class="hist-col-entity">{{ t('library.history.colEntity') }}</th>
              <th scope="col" class="hist-col-snap">{{ t('library.history.colSnap') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="rev in revisions" :key="rowKey(rev)">
              <td class="hist-no">#{{ rev.revNo }}</td>
              <td class="hist-when">{{ rev.when }}</td>
              <td class="hist-op-cell">
                <span class="chip hist-op" :class="opClass(rev)">{{ rev.op }}</span>
              </td>
              <td class="hist-entity">
                <template v-if="rev.entityType === 'instruction'">
                  {{ t('library.history.entityInstruction') }}
                </template>
                <template v-else>
                  {{ rev.entityName }}
                  <!-- The demo attaches the chip in the variant branch
                       only (app.js ~2492): LIB-027 restores variants. -->
                  <span v-if="rev.restored" class="chip hist-restored">
                    {{ t('library.history.restoredChip') }}
                  </span>
                </template>
                <button
                  v-if="rev.op === 'delete'"
                  type="button"
                  class="btn btn-ghost hist-restore-btn"
                >
                  {{ t('library.history.restoreButton') }}
                </button>
              </td>
              <td class="hist-snap-cell">
                <details class="hist-snap">
                  <summary>{{ t('library.history.snapshot') }}</summary>
                  <div class="hist-snap-body">
                    <div class="hist-snap-row">
                      <span class="hist-snap-key">{{ t('library.history.snapName') }}</span>
                      <span>{{ rev.snapshot.name }}</span>
                    </div>
                    <div v-if="rev.snapshot.kind === 'instruction'" class="hist-snap-row">
                      <span class="hist-snap-key">{{ t('library.history.snapState') }}</span>
                      <span>{{ rev.snapshot.state }}</span>
                    </div>
                    <pre v-else class="hist-snap-text">{{ rev.snapshot.text }}</pre>
                  </div>
                </details>
              </td>
            </tr>
          </tbody>
        </table>
        <p class="hist-empty" :hidden="hasRevisions">{{ t('library.history.empty') }}</p>
      </div>

      <div class="modal-actions">
        <button value="close" class="btn btn-ghost">{{ t('library.history.close') }}</button>
      </div>
    </form>
  </BaseModal>
</template>
