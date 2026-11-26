<script setup lang="ts">
// DEV showcase (T-367): the Activity window (LOG-001…006, §7.7) — the
// seed's demo entries newest first, the four filters rendered inert
// (the combining query is the activity card's), the limits hint from
// the seed settings.
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import ActivityDialog from '../../components/activity/ActivityDialog.vue'
import { activityEntries, activityLogLimits, activityProjects } from '../seed'

defineOptions({ inheritAttrs: false })

const { t } = useI18n()

const open = ref(true)
</script>

<template>
  <section class="showcase">
    <h2 class="showcase-title">{{ t('dev.showcase.activity') }}</h2>
    <p class="showcase-hint">{{ t('dev.showcase.activityHint') }}</p>
    <ActivityDialog
      v-model:open="open"
      :entries="activityEntries"
      :projects="activityProjects"
      :log-limits="activityLogLimits"
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
