<script setup lang="ts">
// The variant-contents popover (T-364, MAIN-032): one reusable
// read-only box listing every variant of the row — the main variant
// first — with its FULL text (no 600-character truncation; the box
// clamps to the viewport and long content scrolls inside), the variant
// currently assigned in the cell marked with an "assigned" badge,
// empty variants noted as outputting no text (D-13). Markup facts:
// docs/prototype/index.html 2817–2822 (#variants-popover) and the rows
// app.js builds in openVariantsPopover (app.js 1238–1273). The demo's
// id selector became the .variants-popover class (instance-reusable).
// Closes on outside click and Escape, and the eye toggles it
// (skeleton minimum; scroll-close is T-184/T-355's). Positioning
// computes from this component's own element plus the anchor data —
// container-agnostic (floating.ts; T-169 re-hosts it).
import { computed, onMounted, ref, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'
import { placeFloating, useDismiss, type Anchor } from './floating'

/** One variant line of the popover (the caller derives it from the row). */
export type VariantEntry = {
  /** The variant's display name (the main is named "Main", LIB-002). */
  readonly name: string
  /** The variant's full text ('' when empty). */
  readonly text: string
  /** An empty variant outputs no text (LIB-014/D-13). */
  readonly isEmpty: boolean
  /** The variant currently assigned in the calling cell. */
  readonly current: boolean
}

const props = defineProps<{
  /** The row's variants, the main variant first (MAIN-032). */
  readonly entries: readonly VariantEntry[]
  /** The trigger's bottom-right corner (anchor data, not traversal). */
  readonly anchor: Anchor
}>()

const emit = defineEmits<{ close: [] }>()

const { t } = useI18n()

const box = useTemplateRef<HTMLElement>('box')
/** The clamped fixed position, computed after the box is laid out. */
const position = ref({ left: 0, top: 0 })

onMounted(() => {
  const el = box.value
  if (el !== null) position.value = placeFloating(props.anchor, el, { align: 'start', gap: 6 })
})

const positionStyle = computed(() => ({
  left: `${position.value.left}px`,
  top: `${position.value.top}px`,
}))

useDismiss(
  box,
  () => emit('close'),
  // The eye's toggle click must not close what it just opened (the
  // demo's own hit-test, app.js ~1290).
  '.cell-icon--eye',
)
</script>

<template>
  <div
    ref="box"
    class="variants-popover"
    role="group"
    :aria-label="t('matrix.popover.label')"
    :style="positionStyle"
  >
    <div v-for="entry in entries" :key="entry.name" class="vp-item">
      <div class="vp-head">
        <span class="vp-name">{{ entry.name }}</span>
        <span v-if="entry.current" class="vp-current">{{ t('matrix.popover.assignedBadge') }}</span>
      </div>
      <div v-if="entry.isEmpty" class="vp-empty">{{ t('matrix.popover.emptyNote') }}</div>
      <div v-else class="vp-text">{{ entry.text }}</div>
    </div>
  </div>
</template>
