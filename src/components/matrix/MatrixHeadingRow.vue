<script setup lang="ts">
// One heading row of the matrix (T-364, MAIN-015…023): the mock-up's
// `tr.row-heading` markup (docs/prototype/index.html 369–383) as a
// component — the caret button (the collapse behavior itself is
// UI-004's card; the skeleton renders the reference's static
// aria-expanded="true" state), the section name and the fill cell.
// CONTAINER-AGNOSTIC: props in, one row's content out — the script
// never touches the DOM order, so T-169 can re-host the row in a
// virtualized container.
import { useI18n } from 'vue-i18n'
import type { LibraryHeading } from '../../dev/seed'

defineProps<{
  /** The heading row (name + tree depth). */
  readonly heading: LibraryHeading
  /** The project column count: the fill cell spans them. */
  readonly columnCount: number
}>()

const { t } = useI18n()
</script>

<template>
  <tr class="row-heading">
    <td class="c-check" aria-hidden="true"></td>
    <!-- data-level is the demo's markup fact (the section's `##`/`###`
         depth: tree depth 0 is a level-2 heading, 1 a level-3 one). -->
    <th
      scope="row"
      class="c-name"
      :data-level="heading.depth + 2"
      :style="{ '--depth': heading.depth }"
    >
      <span class="name-wrap"
        ><button
          type="button"
          class="caret-btn"
          aria-expanded="true"
          :aria-label="t('matrix.row.toggleSection', { name: heading.name })"
        ></button
        ><span class="hname">{{ heading.name }}</span></span
      >
    </th>
    <td class="c-fill" :colspan="columnCount"></td>
  </tr>
</template>
