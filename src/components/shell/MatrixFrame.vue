<script setup lang="ts">
// The matrix frame (T-363, MAIN-010): the scroll region with the
// pinned header row and the pinned first columns — table, colgroup,
// thead, an empty tbody — plus the UI-012 empty state. The frame only:
// the rows, the cells and the project-header internals (color chip,
// label, status sub-line) arrive with the cards T-364…T-367 through
// the `#rows` slot; until a parent fills it, the body shows the
// no-instructions empty state. The project columns render the bare
// project names for now — the styled header internals are T-364's,
// together with the stylesheet section that draws them.
import { useI18n } from 'vue-i18n'
import type { Project } from '../../dev/seed'

defineProps<{
  /** The tracked projects: one pinned-width column each. */
  readonly projects: readonly Project[]
}>()

const { t } = useI18n()
</script>

<template>
  <!-- The frame only — the spec's behavioral truth (docs/spec.md) wins
       where it disagrees with the reference. The reference's
       role="region" is dropped: an explicit role would strip <main> of
       its implicit main landmark (the aria-label alone names it). -->
  <main class="matrix-wrap" :aria-label="t('shell.matrix.regionLabel')" tabindex="0">
    <table class="matrix">
      <caption class="visually-hidden">
        {{
          t('shell.matrix.caption')
        }}
      </caption>

      <colgroup>
        <col class="col-check" />
        <col class="col-name" />
        <col v-for="project in projects" :key="project.name" class="col-proj" />
      </colgroup>

      <thead>
        <tr>
          <th scope="col" class="c-check">
            <input type="checkbox" :aria-label="t('shell.matrix.selectAll')" />
          </th>
          <th scope="col" class="c-name">{{ t('shell.matrix.instructionHeader') }}</th>
          <th v-for="project in projects" :key="project.name" scope="col" class="c-proj">
            {{ project.name }}
          </th>
        </tr>
      </thead>

      <!-- The row slot: T-364 fills it with the library rows; the empty
           state below renders only while it is unfilled. -->
      <tbody>
        <slot name="rows"></slot>
      </tbody>
    </table>

    <!-- Empty state (UI-012, no instructions). -->
    <div v-if="!$slots.rows" class="empty-state">
      <p>{{ t('shell.matrix.emptyMessage') }}</p>
      <button type="button" class="btn btn-ghost">{{ t('shell.matrix.emptyAction') }}</button>
    </div>
  </main>
</template>
