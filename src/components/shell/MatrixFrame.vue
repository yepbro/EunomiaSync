<script setup lang="ts">
// The matrix frame (T-363, MAIN-010; rows since T-364): the scroll
// region with the pinned header row and the pinned first columns —
// table, colgroup, thead, and a tbody rendering the seed's library
// rows itself (MatrixHeadingRow / MatrixInstructionRow per row; the
// #rows slot mechanism of the skeleton phase is gone). The header row
// carries the select-all checkbox, the "Instruction" column header
// with the add-section button (MAIN-015; the styles.css 618–621 rule
// targets the `instruction-header` id — a markup fact of the
// reference), and one ProjectColumnHeader per project (the styled
// header internals: color chip, label, status sub-line, edit/preview
// icons, the "⋮"). With no rows the body shows the UI-012
// no-instructions empty state. The frame stays the CONTAINER: the row
// and cell components are container-agnostic (props in, no DOM-order
// traversal), so T-169 can re-host them in a virtualized container.
import { useI18n } from 'vue-i18n'
import ProjectColumnHeader from '../matrix/ProjectColumnHeader.vue'
import MatrixHeadingRow from '../matrix/MatrixHeadingRow.vue'
import MatrixInstructionRow from '../matrix/MatrixInstructionRow.vue'
import type {
  AssignmentMatrix,
  LibraryRow,
  LibraryHeading,
  LibraryInstruction,
  Project,
} from '../../dev/seed'

defineProps<{
  /** The tracked projects: one pinned-width column each. */
  readonly projects: readonly Project[]
  /** The library rows in document order (the seed's `library`). */
  readonly rows: readonly LibraryRow[]
  /** The assignment cells, keyed by instruction then project name. */
  readonly assignments: AssignmentMatrix
}>()

const { t } = useI18n()

const isHeading = (row: LibraryRow): row is LibraryHeading => row.kind === 'heading'
const isInstruction = (row: LibraryRow): row is LibraryInstruction => row.kind === 'instruction'
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
          <th id="instruction-header" scope="col" class="c-name">
            {{ t('shell.matrix.instructionHeader') }}
            <!-- "+" adds a top-level section (MAIN-015): the header
                 stays the drag-drop target for moving a section; the
                 behavior is the library-editing card's. -->
            <button
              type="button"
              class="hbtn hbtn--add"
              :title="t('matrix.addSection')"
              :aria-label="t('matrix.addSection')"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="M11 3h2v8h8v2h-8v8h-2v-8H3v-2h8V3z" />
              </svg>
            </button>
          </th>
          <ProjectColumnHeader v-for="project in projects" :key="project.name" :project="project" />
        </tr>
      </thead>

      <tbody>
        <template v-for="row in rows" :key="row.name">
          <MatrixHeadingRow v-if="isHeading(row)" :heading="row" :column-count="projects.length" />
          <MatrixInstructionRow
            v-else-if="isInstruction(row)"
            :row="row"
            :projects="projects"
            :row-assignments="assignments[row.name]"
          />
        </template>
      </tbody>
    </table>

    <!-- Empty state (UI-012, no instructions). -->
    <div v-if="rows.length === 0" class="empty-state">
      <p>{{ t('shell.matrix.emptyMessage') }}</p>
      <button type="button" class="btn btn-ghost">{{ t('shell.matrix.emptyAction') }}</button>
    </div>
  </main>
</template>
