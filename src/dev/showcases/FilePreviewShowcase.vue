<script setup lang="ts">
// DEV showcase (T-366): the file preview dialog (MAIN-033) on the
// seed's scheme-A project — the target path monospace and the managed
// content built by the spec's contract (ALG-020 headings, ALG-022 no
// instruction names), not the demo's rendering (D-203 (5)).
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import FilePreviewDialog from '../../components/projects/FilePreviewDialog.vue'
import { filePreviewBlocks, filePreviewProjectName, projects } from '../seed'

defineOptions({ inheritAttrs: false })

const { t } = useI18n()

const open = ref(true)

const project = projects.find((p) => p.name === filePreviewProjectName) ?? projects[0]

/** The target path the dialog's monospace line shows: the project's
 * path and base file (the demo's fp-path composition, app.js ~1193). */
const targetPath = computed(() => `${project.path}/${project.baseFile}`)
</script>

<template>
  <section class="showcase">
    <h2 class="showcase-title">{{ t('dev.showcase.projectsPreview') }}</h2>
    <p class="showcase-hint">{{ t('dev.showcase.projectsPreviewHint') }}</p>
    <FilePreviewDialog v-model:open="open" :path="targetPath" :blocks="filePreviewBlocks" />
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
