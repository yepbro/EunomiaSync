<script setup lang="ts">
// DEV showcase (T-365): the variant-deletion confirmation (LIB-023)
// with its affected-projects count derived from the seed matrix — the
// "Spanish" variant of "Language of responses", assigned to one seeded
// project ("yepbro/site").
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import VariantDeleteDialog from '../../components/library/VariantDeleteDialog.vue'
import { assignments } from '../seed'

defineOptions({ inheritAttrs: false })

const { t } = useI18n()

const instructionName = 'Language of responses'
const variantName = 'Spanish'

/** LIB-023's count: the projects whose assignments would move. */
const affectedProjects = computed(
  () =>
    Object.values(assignments[instructionName] ?? {}).filter(
      (assignment) => assignment.state === 'variant' && assignment.variant === variantName,
    ).length,
)

const open = ref(true)
</script>

<template>
  <section class="showcase">
    <h2 class="showcase-title">{{ t('dev.showcase.variantDelete') }}</h2>
    <p class="showcase-hint">{{ t('dev.showcase.variantDeleteHint') }}</p>
    <VariantDeleteDialog
      v-model:open="open"
      :variant-name="variantName"
      :affected-projects="affectedProjects"
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
