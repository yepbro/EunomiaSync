<script setup lang="ts">
// DEV showcase (T-365): the history panel (LIB-026/027) open on the
// seed's demo revisions — the "Commit messages" list the prototype's
// builder produces (src/dev/seed/history.ts): the deleted "Old
// wording" row carries the inert Restore variant button, the
// "Conventional Commits" hide/restore pair sits on top.
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import HistoryDialog from '../../components/library/HistoryDialog.vue'
import { historyInstruction, historyRevisions } from '../seed'

defineOptions({ inheritAttrs: false })

const { t } = useI18n()

const open = ref(true)
</script>

<template>
  <section class="showcase">
    <h2 class="showcase-title">{{ t('dev.showcase.history') }}</h2>
    <p class="showcase-hint">{{ t('dev.showcase.historyHint') }}</p>
    <HistoryDialog
      v-model:open="open"
      :instruction-name="historyInstruction"
      :revisions="historyRevisions"
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
