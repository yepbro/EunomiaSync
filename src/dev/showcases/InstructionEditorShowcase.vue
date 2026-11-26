<script setup lang="ts">
// DEV showcase (T-365): the instruction editor in the library-edit
// mode (UI-003), open on the seed's richest row — "Commit messages"
// (the main variant first with its badge, two alternatives, the star
// and Delete gates with the LIB-024 hint). The static Preview sample
// rides the seed.
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import InstructionEditorDialog from '../../components/library/InstructionEditorDialog.vue'
import { library, previewSample } from '../seed'

// The gallery binds v-model:open on every mounted screen; these roots
// are not dialogs and declare no model, so drop the attribute
// fallthrough (otherwise the binding lands as a stray open="…" attr).
defineOptions({ inheritAttrs: false })

const { t } = useI18n()

const commitRow = library.find((r) => r.kind === 'instruction' && r.name === 'Commit messages')
if (commitRow === undefined || commitRow.kind !== 'instruction') {
  throw new Error('showcase seed drift: no instruction named Commit messages')
}

const open = ref(true)
</script>

<template>
  <section class="showcase">
    <h2 class="showcase-title">{{ t('dev.showcase.instructionEditor') }}</h2>
    <p class="showcase-hint">{{ t('dev.showcase.editorHint') }}</p>
    <InstructionEditorDialog
      v-model:open="open"
      :instruction="commitRow"
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
