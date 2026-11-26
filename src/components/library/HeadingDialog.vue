<script setup lang="ts">
// The add/rename section dialog (T-365, MAIN-015…023, LIB-030/031):
// the mock-up's #heading-dialog (docs/prototype/index.html 2829–2855)
// on the T-363 dialog contract — shared by the header "+", a heading's
// "+" and the heading's pencil. The title follows the `mode` prop the
// way the demo's openHeadingDialog sets it (app.js ~840–845): add →
// "Add section", rename → "Rename section" with the field prefilled
// through `initialName` (the demo's headingNameInput.value fact; the
// parent-context "Add sub-section under …" title is the create flow's
// data, T-185's). No behavior: no validation, no create/rename
// (T-185's family); Cancel/Save are the form's method="dialog" return
// values. The .ps-field/.ps-input/.ps-error/.ps-hint styles live in
// the project-settings card's section (styles.css 1543+) and land
// with it — the D-212 precedent for the shared form idioms.
import { computed, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import BaseModal from '../ui/BaseModal.vue'

const props = withDefaults(
  defineProps<{
    /** Which flow opens the dialog: the title follows it. */
    readonly mode?: 'add' | 'rename'
    /** The field's prefill (the rename flow's current name). */
    readonly initialName?: string
  }>(),
  { mode: 'add', initialName: '' },
)

const { t } = useI18n()

/** The dialog contract (T-363): v-model:open drives the BaseModal. */
const open = defineModel<boolean>('open', { default: false })

const titleId = useId()
const nameId = useId()
const nameErrorId = useId()

const title = computed(() =>
  props.mode === 'rename' ? t('library.heading.titleRename') : t('library.heading.titleAdd'),
)
</script>

<template>
  <BaseModal v-model:open="open" :labelled-by="titleId">
    <form method="dialog" novalidate>
      <h2 :id="titleId">{{ title }}</h2>
      <div class="ps-field">
        <label class="field-label" :for="nameId">{{ t('library.heading.nameLabel') }}</label>
        <input
          :id="nameId"
          class="ps-input"
          type="text"
          autocomplete="off"
          maxlength="200"
          :value="initialName"
          :aria-describedby="nameErrorId"
        />
        <p :id="nameErrorId" class="ps-error" hidden></p>
      </div>
      <p class="ps-hint">{{ t('library.heading.hint') }}</p>
      <div class="modal-actions">
        <button value="cancel" class="btn btn-ghost">{{ t('library.heading.cancel') }}</button>
        <button value="save" class="btn btn-primary">{{ t('library.heading.save') }}</button>
      </div>
    </form>
  </BaseModal>
</template>
