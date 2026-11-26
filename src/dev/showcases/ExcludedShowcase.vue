<script setup lang="ts">
// DEV showcase (T-366): the excluded-paths window (PRJ-013…018) with
// the seed's demo list — path, the human-readable reason badge and
// the "Added <date>" stamp, each row ending in the inert Remove
// action.
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import ExcludedPathsDialog from '../../components/projects/ExcludedPathsDialog.vue'
import { exclusions } from '../seed'

defineOptions({ inheritAttrs: false })

const { t } = useI18n()

const open = ref(true)
</script>

<template>
  <section class="showcase">
    <h2 class="showcase-title">{{ t('dev.showcase.projectsExcluded') }}</h2>
    <p class="showcase-hint">{{ t('dev.showcase.projectsExcludedHint') }}</p>
    <ExcludedPathsDialog v-model:open="open" :rows="exclusions" />
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
