<script setup lang="ts">
// DEV showcase (T-365): the instruction editor in the per-project
// custom mode (MAIN-007, MAIN-034) — the .ie-custom window: the
// variant list and the name fields hidden, the replace/extend radio
// group shown. The data mirrors the demo's custom-cell ADD flow
// (app.js 2680–2692): the cell's replace draft starts as the base
// variant's text, and the "Appended after" label names the instruction
// and base variant the supplement appends to ("Response tone — Main").
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import InstructionEditorDialog from '../../components/library/InstructionEditorDialog.vue'
import { library, previewSample } from '../seed'

defineOptions({ inheritAttrs: false })

const { t } = useI18n()

const toneRow = library.find((r) => r.kind === 'instruction' && r.name === 'Response tone')
if (toneRow === undefined || toneRow.kind !== 'instruction') {
  throw new Error('showcase seed drift: no instruction named Response tone')
}

const open = ref(true)
</script>

<template>
  <section class="showcase">
    <h2 class="showcase-title">{{ t('dev.showcase.editorCustom') }}</h2>
    <p class="showcase-hint">{{ t('dev.showcase.editorCustomHint') }}</p>
    <InstructionEditorDialog
      v-model:open="open"
      mode="custom"
      :instruction="toneRow"
      project-name="quotameter"
      :custom-text="toneRow.text ?? ''"
      custom-base-label="Response tone — Main"
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
