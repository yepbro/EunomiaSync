<script setup lang="ts">
// The file preview dialog (T-366, MAIN-033, UI-002): the mock-up's
// #file-preview-dialog (docs/prototype/index.html 3127–3139) on the
// T-363 dialog contract — BaseModal (size lg) +
// defineModel<boolean>('open'). The project's target path renders
// monospace (.fp-path); the body renders the next sync's managed
// content as typed blocks — template markup, no v-html (the real
// rendering is markdown_preview's, 7.10/SEC-005, a behavior card's).
// The text follows the SPEC, not the demo: section headings that
// have assigned instructions (ALG-020) and the instruction texts,
// WITHOUT instruction names (ALG-022) — the demo's rendering (each
// instruction's name as a `##` heading, no section headings) is not
// followed (D-203 (5)); the seed sample documents the reading. An
// empty column (no blocks) shows the spec's note instead of an empty
// document (MAIN-033). No behavior: Close is the form's
// method="dialog" return value.
import { useId } from 'vue'
import { useI18n } from 'vue-i18n'
import BaseModal from '../ui/BaseModal.vue'
import type { PreviewBlock } from '../../dev/seed'

const props = defineProps<{
  /** The project's target path (the build preview's path fact). */
  readonly path: string
  /** The managed content as typed blocks (the seed's ALG-020/022
   * sample; the real text is project_build_preview's). */
  readonly blocks: readonly PreviewBlock[]
}>()

const { t } = useI18n()

/** The dialog contract (T-363): v-model:open drives the BaseModal. */
const open = defineModel<boolean>('open', { default: false })

const titleId = useId()
</script>

<template>
  <BaseModal v-model:open="open" size="lg" :labelled-by="titleId">
    <form method="dialog" novalidate>
      <h2 :id="titleId">{{ t('projects.preview.title') }}</h2>
      <p class="fp-path">{{ props.path }}</p>
      <div class="fp-body ie-preview">
        <template v-for="(block, index) in props.blocks" :key="index">
          <h2 v-if="block.kind === 'heading' && block.level === 2">{{ block.text }}</h2>
          <h3 v-else-if="block.kind === 'heading' && block.level === 3">{{ block.text }}</h3>
          <h4 v-else-if="block.kind === 'heading' && block.level === 4">{{ block.text }}</h4>
          <p v-else-if="block.kind === 'paragraph'">{{ block.text }}</p>
          <pre v-else-if="block.kind === 'code'"><code>{{ block.text }}</code></pre>
          <ul v-else-if="block.kind === 'list'">
            <li v-for="item in block.items" :key="item">{{ item }}</li>
          </ul>
        </template>
        <!-- MAIN-033: a column with no assignments shows a note. -->
        <p v-if="props.blocks.length === 0" class="fp-note">
          {{ t('projects.preview.emptyNote') }}
        </p>
      </div>
      <div class="modal-actions">
        <button value="close" class="btn btn-primary">{{ t('projects.preview.close') }}</button>
      </div>
    </form>
  </BaseModal>
</template>
