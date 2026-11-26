<script setup lang="ts">
// One help article's typed body (T-367, HLP-001…005): the seed's
// HelpBlock list rendered as template markup — an h4 per section
// heading (the demo's #### blocks, nested under the pane's h3), a
// paragraph per text, a fenced code block, an ol/ul per list. When
// `query` is non-empty, every block's text renders through the
// mark-splitting helper — paragraphs, list items, headings and code
// blocks alike (HLP-001: the matches are <mark>-wrapped in the text,
// the demo's TreeWalker walks every text node); the split runs on
// plain strings — no v-html. The real window renders Markdown through
// markdown_preview (7.10, SEC-005) — a behavior card's; this is the
// typed-block presentation.
import { splitMark } from './highlight'
import type { HelpBlock } from '../../dev/seed'

const props = defineProps<{
  /** The article's typed blocks (the seed's body). */
  readonly blocks: readonly HelpBlock[]
  /** The search query the texts highlight against ('' at rest). */
  readonly query: string
}>()
</script>

<template>
  <template v-for="(block, index) in props.blocks" :key="index">
    <h4 v-if="block.kind === 'heading'">
      <template v-for="(segment, i) in splitMark(block.text, props.query)" :key="i">
        <mark v-if="segment.mark">{{ segment.text }}</mark>
        <template v-else>{{ segment.text }}</template>
      </template>
    </h4>
    <p v-else-if="block.kind === 'paragraph'">
      <template v-for="(segment, i) in splitMark(block.text, props.query)" :key="i">
        <mark v-if="segment.mark">{{ segment.text }}</mark>
        <template v-else>{{ segment.text }}</template>
      </template>
    </p>
    <pre v-else-if="block.kind === 'code'"><code><template
        v-for="(segment, i) in splitMark(block.text, props.query)"
        :key="i"
      ><mark v-if="segment.mark">{{ segment.text }}</mark><template v-else>{{ segment.text }}</template></template></code></pre>
    <ol v-else-if="block.kind === 'list' && block.ordered">
      <li v-for="item in block.items" :key="item">
        <template v-for="(segment, i) in splitMark(item, props.query)" :key="i">
          <mark v-if="segment.mark">{{ segment.text }}</mark>
          <template v-else>{{ segment.text }}</template>
        </template>
      </li>
    </ol>
    <ul v-else-if="block.kind === 'list'">
      <li v-for="item in block.items" :key="item">
        <template v-for="(segment, i) in splitMark(item, props.query)" :key="i">
          <mark v-if="segment.mark">{{ segment.text }}</mark>
          <template v-else>{{ segment.text }}</template>
        </template>
      </li>
    </ul>
  </template>
</template>
