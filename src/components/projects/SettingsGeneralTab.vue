<script setup lang="ts">
// The settings General panel's fields (T-366, PRJ-008…011, 021): the
// mock-up's name/label grid, the description with its counter, the
// palette fieldset and the read-only meta — as a fragment; the
// dialog's .ps-panel flex column spaces the children exactly like the
// reference's direct children. Every value is the seed project's,
// through props: the label chip previews the seed label (PRJ-008's
// live-preview fact, at rest), the description counter shows the seed
// length over the fixed 1000 (the demo formats with toLocaleString —
// the composer's n() here), the palette radios carry the project's
// color checked (DB-040: ring + check mark, never color-only), the
// read-only rows show path, origin wording and the last-sync stamp
// (PRJ-009/021: visible, not editable). No input tracking, no
// validation, no save — T-158's family.
import { computed, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import type { Project } from '../../dev/seed'

const props = defineProps<{
  /** The project the fields show. */
  readonly project: Project
  /** The palette the color fieldset offers (the demo's PS_PALETTE;
   * the tuples' names are the swatches' tooltip/aria-label data). */
  readonly palette: readonly (readonly [hex: string, name: string])[]
}>()

const { t, n } = useI18n()

const nameFieldId = useId()
const nameErrorId = useId()
const labelFieldId = useId()
const labelErrorId = useId()
const descriptionFieldId = useId()

/** The origin wording (the demo's "Manual" / "Found by scan"). */
const originText = computed(() =>
  props.project.origin === 'manual'
    ? t('projects.settings.originManual')
    : t('projects.settings.originScan'),
)
</script>

<template>
  <div class="ps-grid">
    <div class="ps-field">
      <label class="field-label" :for="nameFieldId">{{ t('projects.settings.nameLabel') }}</label>
      <input
        :id="nameFieldId"
        class="ps-input"
        type="text"
        autocomplete="off"
        maxlength="100"
        :value="props.project.name"
        :aria-describedby="nameErrorId"
      />
      <p :id="nameErrorId" class="ps-error" hidden></p>
    </div>

    <div class="ps-field">
      <label class="field-label" :for="labelFieldId">
        {{ t('projects.settings.labelLabel') }}
      </label>
      <div class="ps-label-row">
        <input
          :id="labelFieldId"
          class="ps-input"
          type="text"
          autocomplete="off"
          maxlength="6"
          :value="props.project.label"
          :aria-describedby="labelErrorId"
        />
        <span class="chip ps-label-chip" :title="t('projects.settings.labelPreviewTitle')">
          {{ props.project.label }}
        </span>
      </div>
      <p :id="labelErrorId" class="ps-error" hidden></p>
    </div>
  </div>
  <p class="ps-hint">{{ t('projects.settings.nameHint') }}</p>

  <div class="ps-field">
    <label class="field-label" :for="descriptionFieldId">
      {{ t('projects.settings.descriptionLabel') }}
    </label>
    <textarea
      :id="descriptionFieldId"
      class="ps-textarea"
      rows="3"
      maxlength="1000"
      :placeholder="t('projects.settings.descriptionPlaceholder')"
      :value="props.project.description"
    ></textarea>
    <p class="ps-hint">
      {{ t('projects.settings.descCount', { count: n(props.project.description.length) }) }}
    </p>
  </div>

  <fieldset class="ps-field ps-palette">
    <legend class="field-label">{{ t('projects.settings.colorLabel') }}</legend>
    <div class="ps-swatches">
      <label v-for="[hex, name] in props.palette" :key="hex" class="ps-swatch" :title="name">
        <input
          type="radio"
          name="ps-color"
          :value="hex"
          :checked="hex === props.project.color"
          :aria-label="name"
        />
        <span class="ps-swatch-fill" :style="{ '--c': hex }"></span>
      </label>
    </div>
    <p class="ps-hint">{{ t('projects.settings.paletteHint') }}</p>
  </fieldset>

  <!-- Read-only (PRJ-009/021): path, origin, last sync -->
  <div class="ps-readonly">
    <div class="ps-readonly-row">
      <span class="ps-readonly-key">{{ t('projects.settings.pathKey') }}</span>
      <code class="ps-path">{{ props.project.path }}</code>
    </div>
    <div class="ps-readonly-row">
      <span class="ps-readonly-key">{{ t('projects.settings.addedKey') }}</span>
      <span>{{ originText }}</span>
    </div>
    <div class="ps-readonly-row">
      <span class="ps-readonly-key">{{ t('projects.settings.lastSyncKey') }}</span>
      <span>{{ props.project.syncedAt }}</span>
    </div>
  </div>
</template>
