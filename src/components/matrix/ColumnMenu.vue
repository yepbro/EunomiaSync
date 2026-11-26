<script setup lang="ts">
// The column actions menu (T-364, §7.4 bulk operations, MAIN-033): one
// reusable fixed-position menu opened by a project header's "⋮" button
// — "Preview file…", "Assign all visible", "Clear column" in the
// mock-up's order, plus the inert "Move left" / "Move right" items of
// PRJ-019/020 (docs/spec.md:455; markup only — the move behavior is
// T-179's; D-212 7(i), corrected 2026-11-26). Markup facts:
// docs/prototype/index.html 2806–2815 (#column-menu; the id became the
// .column-menu class, declarations unchanged). Closes on outside click
// and Escape, and the "⋮" toggles it (skeleton minimum; scroll-close
// and the real actions are T-184/T-355's). Positioning computes from
// this component's own element plus the anchor data —
// container-agnostic (floating.ts).
import { computed, onMounted, ref, toRef, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'
import { placeFloating, useDismiss, useExclusiveLayer, type Anchor } from './floating'

/** The menu actions, the ids the owning header receives on select. */
export type ColumnMenuAction = 'preview' | 'assign' | 'clear' | 'moveLeft' | 'moveRight'

const props = defineProps<{
  /** The "⋮" button's bottom-right corner (anchor data, not traversal). */
  readonly anchor: Anchor
  /** This layer's own trigger element (the "⋮" that opened it):
   *  useDismiss ignores only THIS element's clicks. */
  readonly trigger?: Element | null
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
  // PRJ-019/020 (docs/spec.md:455): the column order also changes
  // through the "Move left / Move right" menu — the keyboard
  // alternative to dragging. INERT markup in the skeleton: the ids
  // reach the owner like the three above and nothing writes on them;
  // the move behavior is T-179's (D-212 7(i), corrected 2026-11-26).
  { id: 'moveLeft', label: t('matrix.menu.moveLeft') },
  { id: 'moveRight', label: t('matrix.menu.moveRight') },
])

const onItem = (action: ColumnMenuAction): void => {
  emit('select', action)
  emit('close')
}

// One column menu at a time across the matrix (review H2): mounting
// claims the kind's single slot and closes the layer holding it;
// unmount releases the slot.
useExclusiveLayer('menu', () => emit('close'))

useDismiss(
  box,
  () => emit('close'),
  // The "⋮" toggle click must not close what it just opened — this
  // layer's OWN trigger element, not a class (a class matches every
  // header's "⋮", which left two menus open at once; review H2).
  toRef(() => props.trigger),
)
</script>

<template>
  <div ref="box" class="column-menu" :aria-label="t('matrix.menu.label')" :style="positionStyle">
    <button v-for="item in items" :key="item.id" type="button" @click="onItem(item.id)">
      {{ item.label }}
    </button>
  </div>
</template>
