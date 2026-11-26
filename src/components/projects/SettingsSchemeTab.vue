<script setup lang="ts">
// The settings' sync-settings section (T-366, PRJ-008, PRJ-012): the
// mock-up's section.ps-section inside the General panel — the scheme
// radios, the base-file select and the scheme-conditional blocks.
// The project's scheme decides which block renders (the demo's
// `hidden` toggles become v-if — the same visual): scheme A the
// extra files with their Include flags and Start/End selects (a
// select disabled while its flag is off, PRJ-008), scheme B the
// marker hint with the "Copy marker block" button (D-19), scheme C
// the ready snippet with "Copy" (D-21). The derived names follow the
// base file name (D-20: `<STEM>.local.md` / `<STEM>.project.md`, the
// demo's stemOf as a computed). The snippet composes the D-21
// wording around the derived file name. Copy reports "Copied" /
// "Copy failed" for 1.5 s — a behavior card's (T-160's); the buttons
// are inert.
import { computed, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import type { Project, ProjectScheme } from '../../dev/seed'
import type { CatalogKey } from '../../i18n'

const props = defineProps<{
  /** The project the section shows (scheme, base file, extra files). */
  readonly project: Project
  /** The base-file options (the mock-up's static select options). */
  readonly baseFiles: readonly string[]
}>()

const { t } = useI18n()

const baseFileFieldId = useId()

/** The closed scheme vocabulary, in the mock-up's radio order. */
const schemes: readonly ProjectScheme[] = ['A', 'B', 'C']

/** Each scheme radio's wording key (the letter itself is the option's
 * value — data, rendered as the <strong> lead). */
const schemeTextKey = {
  A: 'projects.settings.schemeA',
  B: 'projects.settings.schemeB',
  C: 'projects.settings.schemeC',
} satisfies Record<ProjectScheme, CatalogKey>

/** The base file's stem: the derived names and the snippet follow it
 * (D-20; the demo's stemOf, app.js ~2982 — a display derivation). */
const stem = computed(() => props.project.baseFile.replace(/\.md$/, ''))

const localName = computed(() => `${stem.value}.local.md`)
const projectFileName = computed(() => `${stem.value}.project.md`)

/** The D-21 snippet line (scheme C's copyable fact). */
const snippet = computed(() => t('projects.settings.snippet', { file: projectFileName.value }))

/** The scheme-conditional block the project's scheme renders. */
const isA = computed(() => props.project.scheme === 'A')
const isB = computed(() => props.project.scheme === 'B')
const isC = computed(() => props.project.scheme === 'C')
</script>

<template>
  <section class="ps-section">
    <h3 class="ps-section-title">{{ t('projects.settings.syncSettingsTitle') }}</h3>

    <fieldset class="ps-field ps-scheme-set">
      <legend class="field-label">{{ t('projects.settings.schemeLabel') }}</legend>
      <div class="ps-schemes">
        <label v-for="s in schemes" :key="s" class="ps-scheme">
          <input type="radio" name="ps-scheme" :value="s" :checked="props.project.scheme === s" />
          <!-- Scheme C's label carries the reference's <code> around
               the STEM placeholder (index.html 3004) — the only code
               element in the three labels, so it composes through
               i18n-t's named slots; A and B are plain text. -->
          <i18n-t v-if="s === 'C'" :keypath="schemeTextKey[s]" tag="span" scope="global">
            <template #letter
              ><strong>{{ s }}</strong></template
            >
            <template #stem
              ><code>{{ t('projects.settings.schemeCToken') }}</code></template
            >
          </i18n-t>
          <span v-else>
            <strong>{{ s }}</strong>
            {{ t(schemeTextKey[s]) }}
          </span>
        </label>
      </div>
    </fieldset>

    <div class="ps-field">
      <label class="field-label" :for="baseFileFieldId">
        {{ t('projects.settings.baseFileLabel') }}
      </label>
      <select :id="baseFileFieldId" class="ps-input ps-select">
        <option
          v-for="file in props.baseFiles"
          :key="file"
          :value="file"
          :selected="file === props.project.baseFile"
        >
          {{ file }}
        </option>
      </select>
    </div>

    <!-- Scheme A: extra files -->
    <div v-if="isA" class="ps-field">
      <span class="field-label">{{ t('projects.settings.extraFilesLabel') }}</span>
      <div class="ps-extra-row">
        <label class="ps-check-row">
          <input type="checkbox" :checked="props.project.extraLocal" />
          {{ t('projects.settings.includeExtra') }} <code>{{ localName }}</code>
        </label>
        <select
          class="ps-input ps-select ps-pos-select"
          :disabled="!props.project.extraLocal"
          :aria-label="t('projects.settings.posLocalLabel')"
        >
          <option value="start" :selected="props.project.extraLocalPos === 'start'">
            {{ t('projects.settings.posStart') }}
          </option>
          <option value="end" :selected="props.project.extraLocalPos === 'end'">
            {{ t('projects.settings.posEnd') }}
          </option>
        </select>
      </div>
      <div class="ps-extra-row">
        <label class="ps-check-row">
          <input type="checkbox" :checked="props.project.extraProject" />
          {{ t('projects.settings.includeExtra') }} <code>{{ projectFileName }}</code>
        </label>
        <select
          class="ps-input ps-select ps-pos-select"
          :disabled="!props.project.extraProject"
          :aria-label="t('projects.settings.posProjectLabel')"
        >
          <option value="start" :selected="props.project.extraProjectPos === 'start'">
            {{ t('projects.settings.posStart') }}
          </option>
          <option value="end" :selected="props.project.extraProjectPos === 'end'">
            {{ t('projects.settings.posEnd') }}
          </option>
        </select>
      </div>
      <i18n-t keypath="projects.settings.extraHint" tag="p" class="ps-hint" scope="global">
        <template #local
          ><code>{{ localName }}</code></template
        >
        <template #gitignore
          ><code>{{ t('projects.settings.gitignoreName') }}</code></template
        >
      </i18n-t>
    </div>

    <!-- Scheme B: the marker pair hint -->
    <div v-else-if="isB" class="ps-field">
      <p class="ps-hint">{{ t('projects.settings.schemeBHint') }}</p>
      <button type="button" class="btn btn-ghost">
        {{ t('projects.settings.copyMarkers') }}
      </button>
    </div>

    <!-- Scheme C: the ready snippet -->
    <div v-else-if="isC" class="ps-field">
      <p class="ps-hint">{{ t('projects.settings.schemeCHint') }}</p>
      <code class="ps-snippet">{{ snippet }}</code>
      <button type="button" class="btn btn-ghost">{{ t('projects.settings.copy') }}</button>
    </div>

    <p class="ps-hint">{{ t('projects.settings.dirtyHint') }}</p>
  </section>
</template>
