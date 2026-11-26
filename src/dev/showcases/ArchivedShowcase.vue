<script setup lang="ts">
// DEV showcase (T-366): the archived-projects window (PRJ-013…018)
// with the seed's demo list — color dot, name, path, "Archived <date>"
// and the origin wording, each row ending in the inert Restore action.
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import ArchivedProjectsDialog from '../../components/projects/ArchivedProjectsDialog.vue'
import { archivedProjects } from '../seed'

defineOptions({ inheritAttrs: false })

const { t } = useI18n()

const open = ref(true)
</script>

<template>
  <section class="showcase">
    <h2 class="showcase-title">{{ t('dev.showcase.projectsArchived') }}</h2>
    <p class="showcase-hint">{{ t('dev.showcase.projectsArchivedHint') }}</p>
    <ArchivedProjectsDialog v-model:open="open" :rows="archivedProjects" />
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
