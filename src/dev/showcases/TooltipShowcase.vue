<script setup lang="ts">
// DEV showcase (T-364): the instruction text tooltip, visible, fed
// with a seed instruction's text (MAIN-006). The component takes only
// `text` and `open` — the 400 ms delay, the hover/focus triggers and
// the viewport clamping are T-187's, so the showcase shows the box at
// its natural position.
import { useI18n } from 'vue-i18n'
import InstructionTextTooltip from '../../components/matrix/InstructionTextTooltip.vue'
import { library } from '../seed'

// The gallery binds v-model:open on every mounted screen; these roots
// are not dialogs and declare no model, so drop the attribute
// fallthrough (otherwise the binding lands as a stray open="…" attr).
defineOptions({ inheritAttrs: false })

const { t } = useI18n()

const commitRow = library.find((r) => r.kind === 'instruction' && r.name === 'Commit messages')
if (commitRow === undefined || commitRow.kind !== 'instruction') {
  throw new Error('showcase seed drift: no instruction named Commit messages')
}
</script>

<template>
  <section class="showcase">
    <h2 class="showcase-title">{{ t('dev.showcase.tooltip') }}</h2>
    <p class="showcase-hint">{{ t('dev.showcase.tooltipHint') }}</p>
    <InstructionTextTooltip :text="commitRow.text ?? ''" :open="true" />
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
