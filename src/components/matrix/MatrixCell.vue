<script setup lang="ts">
// One assignment cell (T-364, MAIN-001…005, MAIN-007, MAIN-032, §11.2,
// NFR-019/020): the mock-up's `td.c-cell` (docs/prototype/index.html
// 425–487; the select/id/icon structure app.js builds in
// buildVariantSelects and refreshCellIcons) as a component. All eight
// §11.2 states render from data, told apart by SHAPE, not color
// (NFR-020):
//   not assigned   — bare checkbox, no select;
//   main variant   — checked + inline select showing "Main";
//   alternative    — checked + select with the variant's name;
//   custom text    — checked + DISABLED select "Custom" + edit/delete;
//   extend custom  — the same plus the dashed "+" chip (.extend-mark);
//   empty variant  — checked + DASHED-border select (the assigned
//                    variant's text is empty, LIB-014/D-13; an
//                    instruction with text: null has an empty main);
//   changed        — any state above, dimmed (.cell-changed), a visible
//                    "(changed)" suffix and a title tooltip;
//   unavailable    — the project column's status is `unavailable`
//                    (D-211 ruling 7: derived here, not stored in the
//                    seed): the cell renders disabled, the header
//                    carries the warning icon.
// Every cell carries the "+" affordance (MAIN-007) — the demo hides the
// icon cluster on unchecked cells, the spec wins ("Every cell, assigned
// or not", §11.2 line 810). Each cell's accessible name is
// "instruction — project: state", including "(changed)" (NFR-019).
// The eye button opens the read-only VariantPopover (MAIN-032) — the
// skeleton's minimum: open/close on click, outside click and Escape
// (scroll-close is T-184/T-355's); no assignment is ever written.
// CONTAINER-AGNOSTIC: props in, one cell's content out; the popover
// positioning computes from the component's own element (floating.ts).
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import VariantPopover from './VariantPopover.vue'
import { anchorOf, type Anchor } from './floating'
import type { Assignment, LibraryInstruction, Project, Variant } from '../../dev/seed'

const props = defineProps<{
  /** The instruction row: its variants and main text feed the select
   *  and the popover. */
  readonly row: LibraryInstruction
  /** The column's project. */
  readonly project: Project
  /** This cell's assignment (the seed matrix's cell). */
  readonly assignment: Assignment
}>()

const { t } = useI18n()

/** An unavailable project's cells render disabled (§11.2 row 8). */
const unavailable = computed(() => props.project.status === 'unavailable')

const isCustom = computed(() => props.assignment.state === 'custom')
const isExtend = computed(
  () => props.assignment.state === 'custom' && props.assignment.mode === 'extend',
)
/** Checked = assigned in any shape (custom cells toggle like the demo's). */
const assigned = computed(() => props.assignment.state !== 'none')

/**
 * The extend cell's base variant display name (MAIN-034: fixed while
 * the cell is custom; `base` names the variant assigned when "+" was
 * pressed, absent on a replace cell and defaulting to the main).
 */
const baseVariantName = computed(() =>
  props.assignment.state === 'custom'
    ? (props.assignment.base ?? t('matrix.cell.mainVariant'))
    : '',
)

/**
 * Empty variant (§11.2 row 6): the assigned variant's text is empty.
 * Reading: the seed's `Variant.text === ''` and an instruction's
 * `text === null` (an empty main) are the two empty shapes — the
 * demo's "empty variant assigned" cell is exactly the latter (D-211
 * ruling 7).
 */
const isEmptyVariant = computed(() => {
  const a = props.assignment
  if (a.state === 'main') return props.row.text === null
  if (a.state === 'variant') {
    const v = props.row.variants.find((candidate: Variant) => candidate.name === a.variant)
    return v !== undefined && v.text === ''
  }
  return false
})

/** The state's screen-reader label (§11.2 column 3), then "(changed)". */
const stateLabel = computed(() => {
  // Unavailable project (§11.2 row 8): the column's state names the
  // cell's label — the disabled cells of the column announce it.
  if (unavailable.value) return t('matrix.cell.stateUnavailable')
  const a = props.assignment
  let label: string
  switch (a.state) {
    case 'none':
      label = t('matrix.cell.stateNone')
      break
    case 'main':
      // Empty variant (§11.2 row 6): the assigned variant's text is
      // empty — its own label, not the main-variant one.
      label = isEmptyVariant.value ? t('matrix.cell.stateEmpty') : t('matrix.cell.stateMain')
      break
    case 'variant':
      label = isEmptyVariant.value
        ? t('matrix.cell.stateEmpty')
        : t('matrix.cell.stateVariant', { variant: a.variant })
      break
    case 'custom':
      label = a.mode === 'extend' ? t('matrix.cell.stateExtend') : t('matrix.cell.stateCustom')
      break
    default: {
      const exhaustive: never = a
      throw new Error(`unknown assignment state: ${String(exhaustive)}`)
    }
  }
  return a.dirty ? `${label} ${t('matrix.cell.changedSuffix')}` : label
})

/** The cell's accessible name: "instruction — project: state" (NFR-019). */
const accessibleName = computed(() =>
  t('matrix.cell.name', {
    instruction: props.row.name,
    project: props.project.name,
    state: stateLabel.value,
  }),
)

/** The select's own label (the demo's aria-label fact, app.js ~267). */
const selectLabel = computed(() => t('matrix.cell.assignedText', { instruction: props.row.name }))

/**
 * The popover's entries (MAIN-032): every variant of the row, the main
 * variant first, with its FULL text (library.ts is the text source —
 * the real app fetches per-variant through `variant_get`), the
 * assigned one marked, empty ones noted.
 */
const popoverEntries = computed(() => [
  {
    name: t('matrix.popover.mainVariantLabel'),
    text: props.row.text ?? '',
    isEmpty: props.row.text === null,
    current: props.assignment.state === 'main',
  },
  ...props.row.variants.map((v: Variant) => ({
    name: v.name,
    text: v.text,
    isEmpty: v.text === '',
    current: props.assignment.state === 'variant' && props.assignment.variant === v.name,
  })),
])

// --- The skeleton's minimum popover behavior: the eye toggles it. ---
const popoverOpen = ref(false)
const popoverAnchor = ref<Anchor>({ left: 0, right: 0, top: 0, bottom: 0 })
/** The eye element the dismiss check ignores (THIS cell's, not a class). */
const popoverTrigger = ref<Element | null>(null)

const togglePopover = (event: MouseEvent): void => {
  if (popoverOpen.value) {
    popoverOpen.value = false
    return
  }
  popoverTrigger.value = event.currentTarget as Element
  popoverAnchor.value = anchorOf(event.currentTarget as Element)
  popoverOpen.value = true
}

const closePopover = (): void => {
  popoverOpen.value = false
}
</script>

<template>
  <td
    class="c-cell"
    :class="{ 'cell-changed': assignment.dirty }"
    :title="assignment.dirty ? t('matrix.cell.changedTitle') : undefined"
  >
    <span class="cell-assign">
      <input
        type="checkbox"
        class="assign"
        :checked="assigned"
        :disabled="unavailable"
        :aria-label="accessibleName"
      />
      <!-- Inline variant select on an assigned cell (MAIN-001); a
           custom cell shows the DISABLED "Custom" select (§11.2 row 4),
           an empty variant adds the dashed border (§11.2 row 6). -->
      <select
        v-if="assigned && !isCustom"
        class="variant-select"
        :class="{ 'variant-select--empty': isEmptyVariant }"
        :disabled="unavailable"
        :aria-label="selectLabel"
      >
        <option value="main" :selected="assignment.state === 'main'">
          {{ t('matrix.cell.mainVariant') }}
        </option>
        <option
          v-for="v in row.variants"
          :key="v.name"
          :value="v.name"
          :selected="assignment.state === 'variant' && assignment.variant === v.name"
        >
          {{ v.name }}
        </option>
      </select>
      <select v-else-if="isCustom" class="variant-select" disabled :aria-label="selectLabel">
        <option value="custom" selected>{{ t('matrix.cell.customLabel') }}</option>
      </select>
      <!-- Extend-mode marker chip (MAIN-034): the dashed "+" — the
           custom text supplements the named base variant. -->
      <span
        v-if="isExtend"
        class="extend-mark"
        :title="t('matrix.custom.extendMarkTitle', { variant: baseVariantName })"
        >+</span
      >
      <!-- Per-cell icon cluster; an unavailable column carries none
           (the demo removes the box, app.js ~2610). -->
      <span v-if="!unavailable" class="cell-icons">
        <button
          v-if="isCustom"
          type="button"
          class="cell-icon cell-icon--edit"
          :title="
            isExtend
              ? t('matrix.custom.editExtend', {
                  variant: baseVariantName,
                  instruction: row.name,
                  project: project.name,
                })
              : t('matrix.custom.edit', { instruction: row.name, project: project.name })
          "
          :aria-label="
            isExtend
              ? t('matrix.custom.editExtend', {
                  variant: baseVariantName,
                  instruction: row.name,
                  project: project.name,
                })
              : t('matrix.custom.edit', { instruction: row.name, project: project.name })
          "
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path
              d="M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.03a.996.996 0 0 0 0-1.41l-2.34-2.34a.996.996 0 0 0-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z"
            />
          </svg>
        </button>
        <button
          v-if="isCustom"
          type="button"
          class="cell-icon cell-icon--delete"
          :title="t('matrix.custom.delete', { instruction: row.name, project: project.name })"
          :aria-label="t('matrix.custom.delete', { instruction: row.name, project: project.name })"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path
              d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z"
            />
          </svg>
        </button>
        <button
          v-if="assigned && !isCustom"
          type="button"
          class="cell-icon cell-icon--eye"
          :aria-expanded="popoverOpen"
          :title="t('matrix.popover.showTexts', { instruction: row.name })"
          :aria-label="t('matrix.popover.showTexts', { instruction: row.name })"
          @click="togglePopover($event)"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path
              d="M12 4C7 4 2.73 7.11 1 11.5 2.73 15.89 7 19 12 19s9.27-3.11 11-7.5C21.27 7.11 17 4 12 4zm0 12.5c-2.76 0-5-2.24-5-5s2.24-5 5-5 5 2.24 5 5-2.24 5-5 5zm0-8c-1.66 0-3 1.34-3 3s1.34 3 3 3 3-1.34 3-3-1.34-3-3-3z"
            />
          </svg>
        </button>
        <!-- The "+" affordance on EVERY cell (MAIN-007): opens the
             per-project custom editor — the editor card's behavior. -->
        <button
          v-if="!isCustom"
          type="button"
          class="cell-icon cell-icon--add"
          :title="t('matrix.custom.add', { instruction: row.name, project: project.name })"
          :aria-label="t('matrix.custom.add', { instruction: row.name, project: project.name })"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M11 3h2v8h8v2h-8v8h-2v-8H3v-2h8V3z" />
          </svg>
        </button>
      </span>
    </span>
    <!-- The visible "(changed)" suffix (§11.2 row 7): color is not the
         only signal (NFR-020). -->
    <span v-if="assignment.dirty" class="cell-changed-mark">
      {{ t('matrix.cell.changedSuffix') }}
    </span>
    <VariantPopover
      v-if="popoverOpen"
      :entries="popoverEntries"
      :anchor="popoverAnchor"
      :trigger="popoverTrigger"
      @close="closePopover"
    />
  </td>
</template>
