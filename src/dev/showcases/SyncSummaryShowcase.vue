<script setup lang="ts">
// DEV showcase (T-367): the sync summary dialog in one of its two
// views — the run summary (the seed's demo run end state, the outcome
// badge and the four counter chips derived from the rows) or the
// pending list (MAIN-026). The gallery registers one wrapper per
// view.
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import SyncSummaryDialog from '../../components/sync/SyncSummaryDialog.vue'
import { syncPendingRows, syncSummaryRows } from '../seed'

const props = defineProps<{
  /** Which view the dialog opens in. */
  readonly view: 'summary' | 'pending'
}>()

defineOptions({ inheritAttrs: false })

const { t } = useI18n()

const open = ref(true)

const titleKey = props.view === 'summary' ? 'dev.showcase.syncSummary' : 'dev.showcase.syncPending'
const hintKey =
  props.view === 'summary' ? 'dev.showcase.syncSummaryHint' : 'dev.showcase.syncPendingHint'
</script>

<template>
  <section class="showcase">
    <h2 class="showcase-title">{{ t(titleKey) }}</h2>
    <p class="showcase-hint">{{ t(hintKey) }}</p>
    <SyncSummaryDialog
      v-model:open="open"
      :view="props.view"
      :rows="syncSummaryRows"
      :pending-rows="syncPendingRows"
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
