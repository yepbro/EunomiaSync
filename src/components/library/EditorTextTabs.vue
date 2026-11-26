<script setup lang="ts">
// The instruction editor's text pane (T-365, UI-003): the mock-up's
// #ie-tabs tablist and its two panels (docs/prototype/index.html
// 2680–2714) — "Edit" and "Preview" as ARIA tabs, the Markdown text
// field with its character counter, and the Preview panel with the
// non-blocking warnings block. The active tab is this component's
// local state (switching by click; the arrow-key/Home/End navigation
// is a feature card's). The Edit panel shows the selected variant's
// text through `:value` — no input tracking (the unsaved-changes
// guard is a behavior card's). The Preview panel renders a STATIC
// sample of typed blocks through props (the real rendering is
// markdown_preview's, 7.10/SEC-005 — no renderer, no v-html here);
// the warnings list stays empty with the mock-up's "no warnings"
// hint. The counter interpolates the live length with the locale
// number format against DB-006's fixed limit.
import { computed, ref, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import type { PreviewBlock } from '../../dev/seed'

const props = defineProps<{
  /** The text field's value (the dialog's selected variant's text). */
  readonly text: string
  /** The text field's placeholder (mode-dependent in the dialog). */
  readonly placeholder: string
  /** The static Preview sample (typed blocks from the seed). */
  readonly previewBlocks: readonly PreviewBlock[]
}>()

const { t, n } = useI18n()

const editTabId = useId()
const previewTabId = useId()
const editPanelId = useId()
const previewPanelId = useId()
const charCountId = useId()
const markerErrorId = useId()

/** Which panel is open; Edit is the mock-up's initial tab. */
const activeTab = ref<'edit' | 'preview'>('edit')

/** "{length} / 262,144" — the count through the locale format (the
 * demo's toLocaleString fact), the limit is DB-006's fixed number. */
const charCount = computed(() => t('library.editor.charCount', { count: n(props.text.length) }))
</script>

<template>
  <div class="ps-tabs" role="tablist" :aria-label="t('library.editor.tabsLabel')">
    <button
      :id="editTabId"
      type="button"
      class="ps-tab"
      role="tab"
      :aria-selected="activeTab === 'edit'"
      :aria-controls="editPanelId"
      :tabindex="activeTab === 'edit' ? 0 : -1"
      @click="activeTab = 'edit'"
    >
      {{ t('library.editor.tabEdit') }}
    </button>
    <button
      :id="previewTabId"
      type="button"
      class="ps-tab"
      role="tab"
      :aria-selected="activeTab === 'preview'"
      :aria-controls="previewPanelId"
      :tabindex="activeTab === 'preview' ? 0 : -1"
      @click="activeTab = 'preview'"
    >
      {{ t('library.editor.tabPreview') }}
    </button>
  </div>

  <div
    :id="editPanelId"
    class="ie-panel"
    role="tabpanel"
    :aria-labelledby="editTabId"
    :hidden="activeTab !== 'edit'"
  >
    <textarea
      class="edit-text"
      rows="8"
      maxlength="262144"
      :aria-label="t('library.editor.textLabel')"
      :aria-describedby="`${charCountId} ${markerErrorId}`"
      :placeholder="placeholder"
      :value="text"
    ></textarea>
    <p :id="markerErrorId" class="ps-error" hidden></p>
    <p class="ie-meta">
      <span :id="charCountId">{{ charCount }}</span>
    </p>
  </div>

  <div
    :id="previewPanelId"
    class="ie-panel"
    role="tabpanel"
    :aria-labelledby="previewTabId"
    :hidden="activeTab !== 'preview'"
  >
    <div class="ie-preview">
      <template v-for="(block, index) in previewBlocks" :key="index">
        <h2 v-if="block.kind === 'heading' && block.level === 2">{{ block.text }}</h2>
        <h3 v-else-if="block.kind === 'heading' && block.level === 3">{{ block.text }}</h3>
        <h4 v-else-if="block.kind === 'heading' && block.level === 4">{{ block.text }}</h4>
        <p v-else-if="block.kind === 'paragraph'">{{ block.text }}</p>
        <pre v-else-if="block.kind === 'code'"><code>{{ block.text }}</code></pre>
        <ul v-else-if="block.kind === 'list'">
          <li v-for="item in block.items" :key="item">{{ item }}</li>
        </ul>
      </template>
    </div>
    <div class="ie-warnings">
      <span class="field-label">{{ t('library.editor.warningsLabel') }}</span>
      <ul class="ie-warnings-list" hidden></ul>
      <p class="ps-hint">{{ t('library.editor.noWarnings') }}</p>
    </div>
  </div>
</template>
