<script setup lang="ts">
// The instruction editor's left pane (T-365, UI-003, LIB-009…016): the
// mock-up's #ie-variants aside (docs/prototype/index.html 2557–2586)
// with the variant rows app.js builds in renderVariantList
// (app.js ~1785–1860). Every row state is data-derived: the main
// variant first with its badge (LIB-021), an empty body marked with
// the matrix's dashed .empty-mark and a dimmed name (LIB-014/D-13);
// the "Make main" star is disabled for the current main and for a
// variant with no name or no body, with the reason as its tooltip
// (LIB-036's UI gate); Delete is disabled for the main variant with
// the "Make another variant main first" hint (LIB-024) — and that
// hint is NOT rendered when the instruction has only the main variant
// (the owner ruling D-207: no alternative text, the button stays
// disabled all the same). The row's select action is the only event:
// selection is the editor dialog's local state (no IPC). The add,
// make-main and delete actions are the behavior cards' (T-189…T-192);
// the Add variant buttons render the mock-up's titles and stay inert.
import { useId } from 'vue'
import { useI18n } from 'vue-i18n'

/** One derived row of the variant list (the dialog derives the rows
 * from the seed's instruction: the implicit main first). */
export type EditorVariantRow = {
  readonly name: string
  readonly text: string
  readonly main: boolean
  /** No body (LIB-014/D-13): the seed main is `text: null`, an
   * alternative an empty string. */
  readonly isEmpty: boolean
}

const props = defineProps<{
  /** The rows in display order — the main variant is always first. */
  readonly variants: readonly EditorVariantRow[]
  /** The selected row's index (the dialog's local state). */
  readonly selected: number
}>()

const emit = defineEmits<{ select: [index: number] }>()

const { t } = useI18n()

const listLabelId = useId()
const addLabelId = useId()

/** LIB-036: a variant with no name or no body cannot become main. */
const isStarBlocked = (row: EditorVariantRow): boolean =>
  row.main || row.name.trim() === '' || row.text === ''

/** The star's tooltip/accessible name: the current main keeps its
 * badge (the demo's "Already the main variant"), an empty variant
 * names the reason, a filled one offers the action. */
const starTitle = (row: EditorVariantRow): string => {
  if (row.main) return t('library.editor.makeMainAlready')
  if (isStarBlocked(row)) return t('library.editor.makeMainEmpty')
  return t('library.editor.makeMain', { name: row.name })
}

/** LIB-024: the main variant's Delete is disabled with the hint —
 * except when it is the only variant, where D-207 hides the hint
 * entirely (no alternative text; the action name stays for screen
 * readers, the reason does not). */
const deleteTitle = (row: EditorVariantRow): string | undefined => {
  if (props.variants.length === 1) return undefined
  if (row.main) return t('library.editor.deleteMainHint')
  return t('library.editor.deleteVariant', { name: row.name })
}

/** The Delete button's accessible name: the hint when it is the
 * disabled main's tooltip, the action name otherwise. */
const deleteLabel = (row: EditorVariantRow): string =>
  row.main && props.variants.length > 1
    ? t('library.editor.deleteMainHint')
    : t('library.editor.deleteVariant', { name: row.name })

const isDeleteDisabled = (row: EditorVariantRow): boolean => row.main || props.variants.length === 1
</script>

<template>
  <aside class="ie-variants" :aria-label="t('library.editor.variantsLabel')">
    <span :id="listLabelId" class="field-label">{{ t('library.editor.variantsLabel') }}</span>
    <div class="ie-variant-list" :aria-labelledby="listLabelId">
      <div v-for="(row, index) in variants" :key="row.name" class="ie-variant-row">
        <button
          type="button"
          :class="row.isEmpty ? 'ie-variant-item ie-variant-empty' : 'ie-variant-item'"
          :aria-pressed="index === props.selected"
          :title="row.isEmpty ? t('library.editor.variantEmptyTitle') : row.name"
          @click="emit('select', index)"
        >
          <span v-if="row.isEmpty" class="empty-mark" aria-hidden="true"></span>
          <span class="ie-variant-label">{{ row.name }}</span>
          <span v-if="row.main" class="ie-main-badge">{{
            t('library.editor.mainVariantName')
          }}</span>
        </button>
        <button
          type="button"
          class="cell-icon cell-icon--main"
          :disabled="isStarBlocked(row)"
          :title="starTitle(row)"
          :aria-label="starTitle(row)"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path
              d="M12 17.27 18.18 21l-1.64-7.03L22 9.24l-7.19-.61L12 2 9.19 8.63 2 9.24l5.46 4.73L5.82 21z"
            />
          </svg>
        </button>
        <button
          type="button"
          class="cell-icon cell-icon--delete"
          :disabled="isDeleteDisabled(row)"
          :title="deleteTitle(row)"
          :aria-label="deleteLabel(row)"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path
              d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z"
            />
          </svg>
        </button>
      </div>
    </div>
    <div class="ie-add-variant">
      <span :id="addLabelId" class="field-label">{{ t('library.editor.addLabel') }}</span>
      <div class="ie-add-row" role="group" :aria-labelledby="addLabelId">
        <button type="button" class="btn btn-ghost" :title="t('library.editor.addCopyTitle')">
          {{ t('library.editor.addCopy') }}
        </button>
        <button type="button" class="btn btn-ghost" :title="t('library.editor.addEmptyTitle')">
          {{ t('library.editor.addEmpty') }}
        </button>
      </div>
    </div>
  </aside>
</template>
