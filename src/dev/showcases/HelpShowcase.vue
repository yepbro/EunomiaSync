<script setup lang="ts">
// DEV showcase (T-367): the Help window (HLP-001…005) with a given
// search query — '' is the at-rest window, a non-empty query shows the
// mark-highlighted list, the match counter and (at zero matches) the
// empty state with the open article staying. The gallery registers one
// wrapper per state.
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import HelpDialog from '../../components/help/HelpDialog.vue'
import { helpArticles } from '../seed'

const props = defineProps<{
  /** The search query the window opens with. */
  readonly query: string
}>()

defineOptions({ inheritAttrs: false })

const { t } = useI18n()

const open = ref(true)

const titleKey = props.query === '' ? 'dev.showcase.help' : 'dev.showcase.helpQuery'
const hintKey = props.query === '' ? 'dev.showcase.helpHint' : 'dev.showcase.helpQueryHint'
</script>

<template>
  <section class="showcase">
    <h2 class="showcase-title">{{ t(titleKey) }}</h2>
    <p class="showcase-hint">{{ t(hintKey) }}</p>
    <HelpDialog v-model:open="open" :articles="helpArticles" :query="props.query" />
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
