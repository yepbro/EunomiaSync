<script setup lang="ts">
// DEV showcase (T-364): the five project-column-header statuses
// (ok, dirty, external, unavailable, deleted) side by side, riding the
// seed projects that carry them (MAIN-028).
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import ProjectColumnHeader from '../../components/matrix/ProjectColumnHeader.vue'
import { projects } from '../seed'

// The gallery binds v-model:open on every mounted screen; these roots
// are not dialogs and declare no model, so drop the attribute
// fallthrough (otherwise the binding lands as a stray open="…" attr).
defineOptions({ inheritAttrs: false })

const { t } = useI18n()

/** One header per status, in the spec's enumeration order. */
const byStatus = computed(() => {
  const wanted = ['ok', 'dirty', 'external', 'unavailable', 'deleted'] as const
  return wanted
    .map((status) => projects.find((p) => p.status === status))
    .filter((p) => p !== undefined)
})
</script>

<template>
  <section class="showcase">
    <h2 class="showcase-title">{{ t('dev.showcase.headerStates') }}</h2>
    <table class="matrix matrix-showcase">
      <thead>
        <tr>
          <ProjectColumnHeader v-for="project in byStatus" :key="project.name" :project="project" />
        </tr>
      </thead>
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
  min-inline-size: 760px;
}
</style>
