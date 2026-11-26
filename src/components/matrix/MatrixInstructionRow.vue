<script setup lang="ts">
// One instruction row of the matrix (T-364, MAIN-001/MAIN-006…009,
// MAIN-015…023, LIB-005…007): the mock-up's instruction-row markup
// (docs/prototype/index.html, e.g. 401–488 for a plain row, 1101–1121
// for a hidden one, 1375–1391 for an empty instruction) as a component
// — the row-selection checkbox, the name cell (edit pencil, empty
// mark, variant chip, hidden chip) and one MatrixCell per project.
// The hide/restore eye button is MAIN-012's behavior card's — the
// demo injects it with JS; the skeleton renders the static markup.
// CONTAINER-AGNOSTIC: props in (the row, the projects, the row's
// assignments), one row's content out — the script never touches the
// DOM order, so T-169 can re-host the row in a virtualized container.
import { useI18n } from 'vue-i18n'
import MatrixCell from './MatrixCell.vue'
import type { Assignment, LibraryInstruction, Project } from '../../dev/seed'

defineProps<{
  /** The instruction row (name, text, variants, hidden flag). */
  readonly row: LibraryInstruction
  /** The tracked projects: one cell each, in column order. */
  readonly projects: readonly Project[]
  /** This row's cells keyed by project name (the seed matrix's row). */
  readonly rowAssignments: Readonly<Record<string, Assignment>>
}>()

const { t } = useI18n()
</script>

<template>
  <tr :class="{ 'row-hidden': row.hidden }">
    <td class="c-check">
      <!-- Row selection (§7.4 bulk operations): renders unchecked — the
           selection state is the matrix behavior card's; the toolbar
           chip counts through App.vue. -->
      <input
        type="checkbox"
        class="row-sel"
        :aria-label="t('matrix.row.selectRow', { name: row.name })"
      />
    </td>
    <th scope="row" class="c-name" :style="{ '--depth': row.depth }">
      <span v-if="row.text === null" class="name-wrap" :title="t('matrix.row.emptyTitle')">
        <button
          type="button"
          class="edit-btn"
          :aria-label="t('matrix.row.editInstruction', { name: row.name })"
        ></button>
        <span class="empty-mark" aria-hidden="true"></span>
        <span class="name name-empty">{{ row.name }}</span>
        <span class="visually-hidden">{{ t('matrix.row.emptyMark') }}</span>
      </span>
      <span v-else class="name-wrap">
        <button
          type="button"
          class="edit-btn"
          :aria-label="t('matrix.row.editInstruction', { name: row.name })"
        ></button>
        <span class="name">{{ row.name }}</span>
        <span
          v-if="row.variants.length > 0"
          class="chip"
          :title="
            t('matrix.row.variantChipTitle', { count: row.variants.length }, row.variants.length)
          "
        >
          {{ row.variants.length }}
        </span>
        <span v-if="row.hidden" class="hidden-chip">{{ t('matrix.row.hiddenChip') }}</span>
      </span>
    </th>
    <MatrixCell
      v-for="project in projects"
      :key="project.name"
      :row="row"
      :project="project"
      :assignment="rowAssignments[project.name]"
    />
  </tr>
</template>
