<script setup lang="ts">
// The instruction editor (T-365, UI-003): the mock-up's #edit-dialog
// (docs/prototype/index.html 2549–2718) on the T-363 dialog contract —
// BaseModal + defineModel<boolean>('open'), size xl. Two-pane window:
// the variant list on the left (EditorVariantList), the name fields,
// the custom-mode radio group and the Edit/Preview tabs on the right.
// Two modes (the `mode` prop): 'instruction' — the library edit, the
// full two-pane window; 'custom' — the per-project custom editor of
// MAIN-007, where the mock-up's .ie-custom class hides the variant
// list and the name fields and shows the MAIN-034 replace/extend
// radio group. No behavior: no validation, no save, no unsaved-changes
// tracking (T-189's family); Save/Cancel are the form's method="dialog"
// return values, History is an inert type="button" (the history panel
// opens through its own screen). The create mode's Section select is
// ported as inert markup (it is the same form in the reference) — the
// picker logic is T-185's, and .ie-section-field stays display:none
// outside .ie-create.
//
// Local UI state only: the selected variant row (switching the fields'
// values) and the replace/extend choice (the placeholder and the
// "Appended after" hint follow it; the draft-resetting of MAIN-034 is
// a behavior card's). Everything else is data-derived: the variant
// rows (main first, LIB-014's empty marks, LIB-036/LIB-024 gates)
// come from the props.
import { computed, ref, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import BaseModal from '../ui/BaseModal.vue'
import EditorVariantList, { type EditorVariantRow } from './EditorVariantList.vue'
import EditorFields from './EditorFields.vue'
import EditorTextTabs from './EditorTextTabs.vue'
import type { LibraryInstruction, PreviewBlock } from '../../dev/seed'

const props = withDefaults(
  defineProps<{
    /** The instruction under edit (the seed row; the real app's
     * library_get/variant_get payloads are the behavior cards'). */
    readonly instruction: LibraryInstruction
    /** 'instruction' — the library edit; 'custom' — MAIN-007's
     * per-project editor (text field + the MAIN-034 radios only). */
    readonly mode?: 'instruction' | 'custom'
    /** Custom mode: the project column the cell belongs to (the title). */
    readonly projectName?: string
    /** Custom mode: the field's text (the cell's custom text; the
     * demo's untouched replace draft is the base variant's text). */
    readonly customText?: string
    /** Custom mode: the "Appended after" label (MAIN-034) — the
     * instruction and base variant the supplement appends to. */
    readonly customBaseLabel?: string
    /** The static Preview sample (typed blocks from the seed). */
    readonly previewBlocks?: readonly PreviewBlock[]
  }>(),
  {
    mode: 'instruction',
    projectName: '',
    customText: '',
    customBaseLabel: '',
    previewBlocks: () => [],
  },
)

const { t } = useI18n()

/** The dialog contract (T-363): v-model:open drives the BaseModal. */
const open = defineModel<boolean>('open', { default: false })

const titleId = useId()
const sectionLabelId = useId()
const modeLabelId = useId()

/** The selected variant row: local UI state (no IPC). */
const selected = ref(0)

/** The MAIN-034 radio choice: local UI state, replace is the default. */
const customMode = ref<'replace' | 'extend'>('replace')

/** The variant rows in display order: the implicit main first, then
 * the seed's alternatives. Empty bodies derive per LIB-014/D-13 (the
 * seed main is `text: null`). */
const variantRows = computed<readonly EditorVariantRow[]>(() => [
  {
    name: t('library.editor.mainVariantName'),
    text: props.instruction.text ?? '',
    main: true,
    isEmpty: props.instruction.text === null,
  },
  ...props.instruction.variants.map((variant) => ({
    name: variant.name,
    text: variant.text,
    main: false,
    isEmpty: variant.text === '',
  })),
])

const selectedRow = computed(() => variantRows.value[selected.value] ?? variantRows.value[0])

const title = computed(() =>
  props.mode === 'custom'
    ? t('library.editor.titleCustom', {
        instruction: props.instruction.name,
        project: props.projectName,
      })
    : t('library.editor.titleEdit', { name: props.instruction.name }),
)

/** The custom mode's .ie-custom class: the mock-up hides the left
 * pane and the name fields through it (styles.css 1204–1211). */
const bodyClass = computed(() => (props.mode === 'custom' ? 'ie-custom' : undefined))

/** The text field's value: the cell's custom text in the custom mode,
 * the selected variant's text otherwise. */
const editorText = computed(() =>
  props.mode === 'custom' ? props.customText : selectedRow.value.text,
)

/** MAIN-034: the placeholder follows the mode — the extend draft
 * starts from empty (its wording says what will be appended). */
const placeholder = computed(() => {
  if (props.mode === 'custom' && customMode.value === 'extend') {
    return t('library.editor.placeholderExtend')
  }
  return props.mode === 'custom'
    ? t('library.editor.placeholderCustom')
    : t('library.editor.placeholder')
})
</script>

<template>
  <BaseModal v-model:open="open" size="xl" :labelled-by="titleId">
    <form method="dialog" novalidate>
      <h2 :id="titleId">{{ title }}</h2>

      <div class="ie-body" :class="bodyClass">
        <EditorVariantList
          :variants="variantRows"
          :selected="selected"
          @select="selected = $event"
        />

        <div class="ie-main">
          <!-- Create mode only (S3 "pick a section", T-185): ported as
               inert markup — the same form carries it in the reference;
               .ie-section-field is display:none outside .ie-create. -->
          <div class="ie-section-field">
            <label class="field-label" :for="sectionLabelId">
              {{ t('library.editor.sectionLabel') }}
            </label>
            <select :id="sectionLabelId" class="ps-select"></select>
          </div>

          <EditorFields :instruction-name="instruction.name" :variant-name="selectedRow.name" />

          <!-- Custom-instruction mode only (.ie-custom): how the
               per-cell text relates to the library variant (MAIN-034). -->
          <div class="ie-mode-group" role="radiogroup" :aria-labelledby="modeLabelId">
            <span :id="modeLabelId" class="field-label">{{ t('library.editor.modeLabel') }}</span>
            <label class="ie-mode-option">
              <input
                type="radio"
                name="ie-custom-mode"
                value="replace"
                :checked="customMode === 'replace'"
                @change="customMode = 'replace'"
              />
              <span class="ie-mode-text">
                <strong>{{ t('library.editor.modeReplace') }}</strong>
                <span class="ie-mode-hint">{{ t('library.editor.modeReplaceHint') }}</span>
              </span>
            </label>
            <label class="ie-mode-option">
              <input
                type="radio"
                name="ie-custom-mode"
                value="extend"
                :checked="customMode === 'extend'"
                @change="customMode = 'extend'"
              />
              <span class="ie-mode-text">
                <strong>{{ t('library.editor.modeExtend') }}</strong>
                <span class="ie-mode-hint">{{ t('library.editor.modeExtendHint') }}</span>
              </span>
            </label>
            <p class="ie-mode-base" :hidden="customMode !== 'extend'">
              {{ t('library.editor.modeBase', { base: customBaseLabel }) }}
            </p>
          </div>

          <EditorTextTabs
            :text="editorText"
            :placeholder="placeholder"
            :preview-blocks="previewBlocks"
          />
        </div>
      </div>

      <div class="modal-actions">
        <button type="button" class="btn btn-ghost">{{ t('library.editor.history') }}</button>
        <span class="spacer"></span>
        <button value="cancel" class="btn btn-ghost">{{ t('library.editor.cancel') }}</button>
        <button value="save" class="btn btn-primary">{{ t('library.editor.save') }}</button>
      </div>
    </form>
  </BaseModal>
</template>
