<script setup lang="ts">
// DEV showcase (T-364): one matrix cell per §11.2 state in a mini
// table, so the reviewer and the owner can see every state (and its
// shape-only distinction, NFR-020) side by side. The states ride REAL
// seed rows; the assignments are fabricated here (data, not behavior).
import { useI18n } from 'vue-i18n'
import MatrixCell from '../../components/matrix/MatrixCell.vue'
import { library, projects } from '../seed'
import type { Assignment, LibraryInstruction, Project } from '../seed'

// The gallery binds v-model:open on every mounted screen; these roots
// are not dialogs and declare no model, so drop the attribute
// fallthrough (otherwise the binding lands as a stray open="…" attr).
defineOptions({ inheritAttrs: false })

const { t } = useI18n()

const byName = (name: string): LibraryInstruction => {
  const row = library.find((r) => r.kind === 'instruction' && r.name === name)
  if (row === undefined || row.kind !== 'instruction') {
    throw new Error(`showcase seed drift: no instruction named ${name}`)
  }
  return row
}

const projectByName = (name: string): Project => {
  const p = projects.find((candidate) => candidate.name === name)
  if (p === undefined) throw new Error(`showcase seed drift: no project named ${name}`)
  return p
}

const responseTone = byName('Response tone')
const language = byName('Language of responses')
const naming = byName('Naming conventions')
const preCommit = byName('Pre-commit checks')
const yepbro = projectByName('yepbro/site')
const dejaVu = projectByName('utils/deja-vu')

const states: readonly {
  readonly labelKey: string
  readonly row: LibraryInstruction
  readonly project: Project
  readonly assignment: Assignment
}[] = [
  {
    labelKey: 'matrix.cell.stateNone',
    row: responseTone,
    project: yepbro,
    assignment: { state: 'none', dirty: false },
  },
  {
    labelKey: 'matrix.cell.stateMain',
    row: responseTone,
    project: yepbro,
    assignment: { state: 'main', dirty: false },
  },
  {
    labelKey: 'matrix.cell.stateVariant',
    row: language,
    project: yepbro,
    assignment: { state: 'variant', variant: 'Spanish', dirty: false },
  },
  {
    labelKey: 'matrix.cell.stateCustom',
    row: responseTone,
    project: yepbro,
    assignment: { state: 'custom', mode: 'replace', dirty: false },
  },
  {
    labelKey: 'matrix.cell.stateExtend',
    row: naming,
    project: yepbro,
    assignment: { state: 'custom', mode: 'extend', base: 'Strict', dirty: false },
  },
  {
    labelKey: 'matrix.cell.stateEmpty',
    row: preCommit,
    project: yepbro,
    assignment: { state: 'main', dirty: false },
  },
  {
    labelKey: 'matrix.cell.changedSuffix',
    row: responseTone,
    project: yepbro,
    assignment: { state: 'main', dirty: true },
  },
  {
    labelKey: 'matrix.header.subUnavailable',
    row: responseTone,
    project: dejaVu,
    assignment: { state: 'none', dirty: false },
  },
]
</script>

<template>
  <section class="showcase">
    <h2 class="showcase-title">{{ t('dev.showcase.cellStates') }}</h2>
    <table class="matrix matrix-showcase">
      <tbody>
        <tr v-for="state in states" :key="state.labelKey">
          <td class="showcase-label">{{ t(state.labelKey) }}</td>
          <MatrixCell :row="state.row" :project="state.project" :assignment="state.assignment" />
        </tr>
      </tbody>
    </table>
  </section>
</template>

<style scoped>
/* Dev-only wrapper styling (scoped, like the gallery launcher). */
.showcase {
  margin: 16px;
}

.showcase-title {
  font-size: 14px;
  margin: 0 0 8px;
}

.matrix-showcase {
  min-inline-size: 480px;
}

.showcase-label {
  min-inline-size: 220px;
  color: var(--ink-muted);
  font-size: 11px;
}
</style>
