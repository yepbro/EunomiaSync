<script setup lang="ts">
// The instruction text tooltip (T-364, MAIN-006): one reusable
// positioned element shared by every row — the mock-up's
// #text-tooltip (docs/prototype/index.html 3940; the id became the
// .text-tooltip class, declarations unchanged). The component is pure
// presentation: props `text` and `open` only. MAIN-006's 400 ms delay,
// hover/focus triggers and viewport clamping are T-187's; the 600
// character ellipsis truncation rides here (the demo's
// TOOLTIP_MAX_CHARS fact, app.js 527/537–538), so a caller cannot
// render an unbounded box.
import { computed } from 'vue'

const props = defineProps<{
  /** The instruction's text (the main variant's, MAIN-006). */
  readonly text: string
  /** Visibility — the delay/trigger wiring is T-187's. */
  readonly open: boolean
}>()

/** MAIN-006: the first 600 characters, an ellipsis when truncated. */
const TOOLTIP_MAX_CHARS = 600

const truncated = computed(() =>
  props.text.length > TOOLTIP_MAX_CHARS ? `${props.text.slice(0, TOOLTIP_MAX_CHARS)}…` : props.text,
)
</script>

<template>
  <div v-if="open" class="text-tooltip" role="tooltip">{{ truncated }}</div>
</template>
