<script setup lang="ts">
// DEV showcase (T-364): the column actions menu, open next to a
// trigger styled like the header's "⋮" button. The anchor computes
// from the showcase's own trigger (the same data the real header
// hands the menu); outside click, Escape and the trigger toggle close
// and reopen it.
import { onMounted, ref, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'
import ColumnMenu from '../../components/matrix/ColumnMenu.vue'
import { anchorOf, type Anchor } from '../../components/matrix/floating'

// The gallery binds v-model:open on every mounted screen; these roots
// are not dialogs and declare no model, so drop the attribute
// fallthrough (otherwise the binding lands as a stray open="…" attr).
defineOptions({ inheritAttrs: false })

const { t } = useI18n()

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
    <h2 class="showcase-title">{{ t('dev.showcase.columnMenu') }}</h2>
    <p class="showcase-hint">{{ t('dev.showcase.menuHint') }}</p>
    <button
      ref="trigger"
      type="button"
      class="cell-icon col-menu-btn showcase-trigger"
      :aria-haspopup="true"
      :aria-expanded="open"
      :aria-label="t('matrix.menu.label')"
      @click="toggle"
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path
          d="M12 8c1.1 0 2-.9 2-2s-.9-2-2-2-2 .9-2 2 .9 2 2 2zm0 2c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm0 6c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2z"
        />
      </svg>
    </button>
    <ColumnMenu v-if="open" :anchor="anchor" :trigger="trigger" @close="open = false" />
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
