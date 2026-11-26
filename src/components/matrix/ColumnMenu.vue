<script setup lang="ts">
// The column actions menu (T-364, §7.4 bulk operations, MAIN-033): one
// reusable fixed-position menu opened by a project header's "⋮" button
// — "Preview file…", "Assign all visible", "Clear column" in the
// mock-up's order. Markup facts: docs/prototype/index.html 2806–2815
// (#column-menu; the id became the .column-menu class, declarations
// unchanged). Closes on outside click and Escape, and the "⋮" toggles
// it (skeleton minimum; scroll-close and the real actions are
// T-184/T-355's). Positioning computes from this component's own
// element plus the anchor data — container-agnostic (floating.ts).
import { computed, onMounted, ref, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'
import { placeFloating, useDismiss, type Anchor } from './floating'

/** The menu actions, the ids the owning header receives on select. */
export type ColumnMenuAction = 'preview' | 'assign' | 'clear'

const props = defineProps<{
  /** The "⋮" button's bottom-right corner (anchor data, not traversal). */
  readonly anchor: Anchor
}>()

const emit = defineEmits<{ close: []; select: [action: ColumnMenuAction] }>()

const { t } = useI18n()

const box = useTemplateRef<HTMLElement>('box')
/** The clamped fixed position, computed after the box is laid out. */
const position = ref({ left: 0, top: 0 })

onMounted(() => {
  const el = box.value
  if (el !== null) {
    position.value = placeFloating(props.anchor, el, { align: 'end', gap: 4 })
    // The demo moves focus to the first item on open (app.js ~1115).
    el.querySelector('button')?.focus()
  }
})

const positionStyle = computed(() => ({
  left: `${position.value.left}px`,
  top: `${position.value.top}px`,
}))

const items = computed<readonly { readonly id: ColumnMenuAction; readonly label: string }[]>(() => [
  { id: 'preview', label: t('matrix.menu.preview') },
  { id: 'assign', label: t('matrix.menu.assignAll') },
  { id: 'clear', label: t('matrix.menu.clearColumn') },
])

const onItem = (action: ColumnMenuAction): void => {
  emit('select', action)
  emit('close')
}

useDismiss(
  box,
  () => emit('close'),
  // The "⋮" toggle click must not close what it just opened (the
  // demo's own hit-test, app.js ~1144).
  '.col-menu-btn',
)
</script>

<template>
  <div ref="box" class="column-menu" :aria-label="t('matrix.menu.label')" :style="positionStyle">
    <button v-for="item in items" :key="item.id" type="button" @click="onItem(item.id)">
      {{ item.label }}
    </button>
  </div>
</template>
