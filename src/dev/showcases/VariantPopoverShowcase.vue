<script setup lang="ts">
// DEV showcase (T-364): the variant-contents popover, open next to an
// eye trigger. The entries ride the seed's "Commit messages" row
// (two variants with full texts, the alternative currently assigned —
// the demo's variant-btn fact), so the badge and the full-text
// rendering are visible without any behavior.
import { onMounted, ref, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'
import VariantPopover from '../../components/matrix/VariantPopover.vue'
import { anchorOf, type Anchor } from '../../components/matrix/floating'
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

/** The popover's entries: the main variant first, then alternatives. */
const entries = [
  {
    name: t('matrix.popover.mainVariantLabel'),
    text: commitRow.text ?? '',
    isEmpty: commitRow.text === null,
    current: false,
  },
  ...commitRow.variants.map((v) => ({
    name: v.name,
    text: v.text,
    isEmpty: v.text === '',
    current: v.name === 'Conventional Commits',
  })),
]

const open = ref(true)
const anchor = ref<Anchor>({ left: 0, right: 0, top: 0, bottom: 0 })
const trigger = useTemplateRef<HTMLButtonElement>('trigger')

onMounted(() => {
  const el = trigger.value
  if (el !== null) anchor.value = anchorOf(el)
})

const toggle = (): void => {
  const el = trigger.value
  if (el !== null) anchor.value = anchorOf(el)
  open.value = !open.value
}
</script>

<template>
  <section class="showcase">
    <h2 class="showcase-title">{{ t('dev.showcase.variantPopover') }}</h2>
    <p class="showcase-hint">{{ t('dev.showcase.popoverHint') }}</p>
    <button
      ref="trigger"
      type="button"
      class="cell-icon cell-icon--eye showcase-trigger"
      :aria-expanded="open"
      :aria-label="t('matrix.popover.showTexts', { instruction: commitRow.name })"
      @click="toggle"
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path
          d="M12 4C7 4 2.73 7.11 1 11.5 2.73 15.89 7 19 12 19s9.27-3.11 11-7.5C21.27 7.11 17 4 12 4zm0 12.5c-2.76 0-5-2.24-5-5s2.24-5 5-5 5 2.24 5 5-2.24 5-5 5zm0-8c-1.66 0-3 1.34-3 3s1.34 3 3 3 3-1.34 3-3-1.34-3-3-3z"
        />
      </svg>
    </button>
    <VariantPopover v-if="open" :entries="entries" :anchor="anchor" @close="open = false" />
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

.showcase-trigger {
  background: var(--surface-2);
  border: 1px solid var(--line);
  border-radius: var(--radius-sm);
  color: var(--ink);
}
</style>
