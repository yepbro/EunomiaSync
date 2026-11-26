<script setup lang="ts">
// DEV showcase (T-365): the instruction editor on the seed's empty
// single-variant instruction — "Pre-commit checks" (text: null, no
// alternatives). One row, the main: dashed-marked and dimmed (LIB-014),
// its star disabled with the empty-variant reason (LIB-036) and its
// Delete disabled WITHOUT the hint — the D-207 ruling (a single-variant
// instruction renders no "Make another variant main first" text).
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import InstructionEditorDialog from '../../components/library/InstructionEditorDialog.vue'
import { library, previewSample } from '../seed'

defineOptions({ inheritAttrs: false })

const { t } = useI18n()

const emptyRow = library.find((r) => r.kind === 'instruction' && r.name === 'Pre-commit checks')
if (emptyRow === undefined || emptyRow.kind !== 'instruction') {
  throw new Error('showcase seed drift: no instruction named Pre-commit checks')
}

const open = ref(true)
</script>

<template>
  <section class="showcase">
    <h2 class="showcase-title">{{ t('dev.showcase.editorEmpty') }}</h2>
    <p class="showcase-hint">{{ t('dev.showcase.editorEmptyHint') }}</p>
    <InstructionEditorDialog
      v-model:open="open"
      :instruction="emptyRow"
      :preview-blocks="previewSample"
    />
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

.showcase-hint {
  color: var(--ink-muted);
  font-size: 11px;
  margin: 0 0 12px;
}
</style>
