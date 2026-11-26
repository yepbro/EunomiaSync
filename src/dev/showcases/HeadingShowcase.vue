<script setup lang="ts">
// DEV showcase (T-365): the add/rename section dialog (MAIN-015…023,
// LIB-030/031), prefilled with a seed heading's name — the rename
// flow's fact (the pencil opens it prefilled).
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import HeadingDialog from '../../components/library/HeadingDialog.vue'

defineOptions({ inheritAttrs: false })

const { t } = useI18n()

const open = ref(true)
</script>

<template>
  <section class="showcase">
    <h2 class="showcase-title">{{ t('dev.showcase.heading') }}</h2>
    <p class="showcase-hint">{{ t('dev.showcase.headingHint') }}</p>
    <HeadingDialog v-model:open="open" initial-name="Code comments" />
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
