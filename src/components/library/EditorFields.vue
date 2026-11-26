<script setup lang="ts">
// The instruction editor's name fields (T-365, UI-003, LIB-002): the
// mock-up's #ie-fields block (docs/prototype/index.html 2616–2640) —
// the separate "Instruction name" and "Variant name" inputs with
// their inline error paragraphs. No validation and no input tracking
// here: the values arrive per render from the dialog's selected
// variant (the fields re-render on selection), the error paragraphs
// keep the mock-up's markup hidden until the validation card lands
// (T-189). The .ps-input/.ps-error styles live in src/styles/form.css
// (the shared form primitives).
import { useId } from 'vue'
import { useI18n } from 'vue-i18n'

defineProps<{
  /** The instruction name field's value (the seed's name). */
  readonly instructionName: string
  /** The variant name field's value (the selected variant's name). */
  readonly variantName: string
}>()

const { t } = useI18n()

const nameId = useId()
const nameErrorId = useId()
const variantNameId = useId()
const variantNameErrorId = useId()
</script>

<template>
  <div class="ie-fields">
    <div class="ie-field">
      <label class="field-label" :for="nameId">{{ t('library.editor.nameLabel') }}</label>
      <input
        :id="nameId"
        class="ps-input"
        type="text"
        autocomplete="off"
        maxlength="200"
        :value="instructionName"
        :aria-describedby="nameErrorId"
      />
      <p :id="nameErrorId" class="ps-error" hidden></p>
    </div>
    <div class="ie-field">
      <label class="field-label" :for="variantNameId">
        {{ t('library.editor.variantNameLabel') }}
      </label>
      <input
        :id="variantNameId"
        class="ps-input"
        type="text"
        autocomplete="off"
        maxlength="100"
        :value="variantName"
        :aria-describedby="variantNameErrorId"
      />
      <p :id="variantNameErrorId" class="ps-error" hidden></p>
    </div>
  </div>
</template>
