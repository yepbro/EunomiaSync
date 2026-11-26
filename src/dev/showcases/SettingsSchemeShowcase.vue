<script setup lang="ts">
// DEV showcase (T-366): the project settings window (PRJ-008…012, 021)
// on the seed project of a given scheme, so every scheme-conditional
// block is visible from the gallery: A the extra files with their
// disabled-while-off position selects, B the marker hint, C the ready
// snippet. The Backups tab carries the seed rows of the same project.
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import ProjectSettingsDialog from '../../components/projects/ProjectSettingsDialog.vue'
import { backupsByProject, projectPalette, projects } from '../seed'
import type { ProjectScheme } from '../seed'

/** The base-file options (the mock-up's static select options). */
const BASE_FILES: readonly string[] = ['AGENTS.md', 'CLAUDE.md']

const props = defineProps<{
  /** Which seed project to open: the first with this scheme. */
  readonly scheme: ProjectScheme
}>()

defineOptions({ inheritAttrs: false })

const { t } = useI18n()

const open = ref(true)

const project = computed(() => {
  const found = projects.find((p) => p.scheme === props.scheme)
  return found ?? projects[0]
})

const backups = computed(() => backupsByProject[project.value.name] ?? [])
</script>

<template>
  <section class="showcase">
    <h2 class="showcase-title">
      {{ t('dev.showcase.projectsSettings', { scheme: props.scheme }) }}
    </h2>
    <p class="showcase-hint">
      {{ t('dev.showcase.projectsSettingsHint', { name: project.name }) }}
    </p>
    <ProjectSettingsDialog
      v-model:open="open"
      :project="project"
      :palette="projectPalette"
      :base-files="BASE_FILES"
      :backups="backups"
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
