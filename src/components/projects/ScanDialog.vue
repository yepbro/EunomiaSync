<script setup lang="ts">
// The scan window (T-366, PRJ-002…007, NFR-015): the mock-up's
// #scan-dialog (docs/prototype/index.html 3221–3282) on the T-363
// dialog contract — BaseModal (size lg, the `.scan-dialog` class
// hooking the section's height override) + defineModel<boolean>('open').
// Everything visible is derived from props (the seed carries the
// reference's demo walk END state — counters, current path, the three
// result groups; the walk itself is the demo's timer script and is
// never ported, D-206). Both phase blocks render at rest: the demo's
// phase switching was JS (the `hidden` attributes are its toggles);
// showing one phase over the other is the scan card's behavior
// (T-160's family). The configuration line interpolates the settings
// seed's depth and exclusions — the same source the demo reads
// (app.js ~3450). The indeterminate bar is a native <progress>
// without a value (the card's reading); the browser paints its own
// indeterminate state. "Add selected" is enabled at rest: the New
// rows arrive checked (PRJ-006). No behavior: scanning, cancel,
// "Scan again" and the add flow are behavior cards'; the buttons are
// the mock-up's inert type="button" controls.
import { useId } from 'vue'
import { useI18n } from 'vue-i18n'
import BaseModal from '../ui/BaseModal.vue'
import type { ScanKnownRow, ScanNewRow, ScanSkipReason, ScanSkipRow } from '../../dev/seed'

const props = defineProps<{
  /** The display form of the scanned root (the demo's "~/projects"). */
  readonly root: string
  /** The root's expanded form (the title's tooltip data). */
  readonly rootPath: string
  /** The active scan depth (the settings seed's scan.maxDepth). */
  readonly maxDepth: number
  /** The active exclusion names (the settings seed's scan.excludeNames). */
  readonly excludeNames: readonly string[]
  /** The "Folders checked" counter (the demo walk's end value). */
  readonly foldersChecked: number
  /** The "Repositories found" counter (the demo walk's end value). */
  readonly reposFound: number
  /** The current path line (the demo walk's last step). */
  readonly currentPath: string
  /** The three result groups (PRJ-002). */
  readonly newRows: readonly ScanNewRow[]
  readonly knownRows: readonly ScanKnownRow[]
  readonly skippedRows: readonly ScanSkipRow[]
}>()

const { t } = useI18n()

/** The dialog contract (T-363): v-model:open drives the BaseModal. */
const open = defineModel<boolean>('open', { default: false })

const titleId = useId()
const newHeadingId = useId()
const knownHeadingId = useId()
const skippedHeadingId = useId()

/** The reason badge's wording key (the wording is catalog copy keyed
 * by the closed reason vocabulary, not seed data). */
const reasonKey: Record<ScanSkipReason, string> = {
  excluded: 'projects.scan.reason.excluded',
  submodule: 'projects.scan.reason.submodule',
  noAccess: 'projects.scan.reason.noAccess',
  nonUtf8: 'projects.scan.reason.nonUtf8',
  archived: 'projects.scan.reason.archived',
  inExclusions: 'projects.scan.reason.inExclusions',
}

/** The reason's fix hint key (PRJ-002: each skipped row carries a
 * short hint on how to fix it). */
const hintKey: Record<ScanSkipReason, string> = {
  excluded: 'projects.scan.skipHint.excluded',
  submodule: 'projects.scan.skipHint.submodule',
  noAccess: 'projects.scan.skipHint.noAccess',
  nonUtf8: 'projects.scan.skipHint.nonUtf8',
  archived: 'projects.scan.skipHint.archived',
  inExclusions: 'projects.scan.skipHint.inExclusions',
}

/** The badge tint class per reason (the demo's `tone` field is
 * presentation; the seed carries the reason alone). Neutral reasons
 * map to '' — the default .chip look, the text carries the meaning. */
const reasonClass: Record<ScanSkipReason, string> = {
  excluded: 'scan-reason--warn',
  submodule: 'scan-reason--warn',
  noAccess: 'scan-reason--bad',
  nonUtf8: 'scan-reason--bad',
  archived: '',
  inExclusions: '',
}
</script>

<template>
  <BaseModal v-model:open="open" size="lg" class="scan-dialog" :labelled-by="titleId">
    <form method="dialog" novalidate>
      <h2 :id="titleId" :title="t('projects.scan.rootHint', { root: props.rootPath })">
        {{ t('projects.scan.title', { root: props.root }) }}
      </h2>
      <p class="modal-text" :title="t('projects.scan.configHint')">
        {{
          t('projects.scan.config', {
            depth: props.maxDepth,
            excluded: props.excludeNames.join(', '),
          })
        }}
      </p>

      <!-- Phase 1: progress (the demo started it on open) -->
      <div class="scan-progress">
        <div class="scan-counters" aria-live="polite">
          <span>
            <strong>{{ props.foldersChecked }}</strong>
            {{ t('projects.scan.foldersChecked') }}
          </span>
          <span>
            <strong>{{ props.reposFound }}</strong>
            {{ t('projects.scan.reposFound') }}
          </span>
        </div>
        <code class="scan-path">{{ props.currentPath }}</code>
        <!-- Indeterminate: no value attribute (NFR-015's visible
             progress; the counters carry the live region). -->
        <progress class="scan-bar" aria-hidden="true"></progress>
      </div>

      <!-- Phase 2: results in three groups (PRJ-002) -->
      <div class="scan-results">
        <section class="scan-group" :aria-labelledby="newHeadingId">
          <h3 :id="newHeadingId">
            {{ t('projects.scan.groupNew') }} <span class="chip">{{ props.newRows.length }}</span>
          </h3>
          <div class="scan-group-list">
            <label v-for="row in props.newRows" :key="row.path" class="scan-row">
              <input type="checkbox" class="scan-check" checked />
              <span class="scan-row-name">{{ row.name }}</span>
              <code class="scan-row-path">{{ row.path }}</code>
            </label>
          </div>
        </section>
        <section class="scan-group" :aria-labelledby="knownHeadingId">
          <h3 :id="knownHeadingId">
            {{ t('projects.scan.groupKnown') }}
            <span class="chip">{{ props.knownRows.length }}</span>
          </h3>
          <div class="scan-group-list">
            <div v-for="row in props.knownRows" :key="row.path" class="scan-row scan-row-known">
              <span class="scan-row-name">{{ row.name }}</span>
              <code class="scan-row-path">{{ row.path }}</code>
            </div>
          </div>
        </section>
        <section class="scan-group" :aria-labelledby="skippedHeadingId">
          <h3 :id="skippedHeadingId">
            {{ t('projects.scan.groupSkipped') }}
            <span class="chip">{{ props.skippedRows.length }}</span>
          </h3>
          <div class="scan-group-list">
            <div v-for="row in props.skippedRows" :key="row.path" class="scan-row scan-row-skip">
              <span class="chip scan-reason" :class="reasonClass[row.reason]">
                {{ t(reasonKey[row.reason]) }}
              </span>
              <code class="scan-row-path">{{ row.path }}</code>
              <span class="scan-reason-hint">{{ t(hintKey[row.reason]) }}</span>
            </div>
          </div>
        </section>
      </div>

      <div class="modal-actions">
        <button type="button" class="btn btn-ghost">{{ t('projects.scan.scanAgain') }}</button>
        <span class="spacer"></span>
        <button type="button" class="btn btn-ghost">{{ t('projects.scan.cancel') }}</button>
        <button type="button" class="btn btn-primary">{{ t('projects.scan.addSelected') }}</button>
      </div>
    </form>
  </BaseModal>
</template>
