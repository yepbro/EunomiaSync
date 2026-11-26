<script setup lang="ts">
// The Settings window's "Defaults for new projects" section (T-367,
// DEF-001, DEF-005): the mock-up's first set-body section
// (docs/prototype/index.html 3373–3445) — the scheme radios, the
// base-file select and the extra-files block. The extra-files block is
// visible only for scheme A (DEF-001: the demo's hidden toggle becomes
// v-if, the project settings precedent); the derived
// `<STEM>.local.md`/`<STEM>.project.md` names follow the base file
// (D-20, the demo's stemOf as a computed) and a position select is
// disabled while its Include flag is off (the demo's
// syncSetExtraRows). The scheme labels reuse the project settings'
// wording keys (one wording, two screens — the D-215 precedent);
// scheme C's <code> token composes through i18n-t's slots. No
// behavior: the radios and selects carry the seed values and stay
// inert.
import { computed, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import type { ProjectScheme, Settings } from '../../dev/seed'
import type { CatalogKey } from '../../i18n'

const props = defineProps<{
  /** The defaults for new projects (the seed's default.* group). */
  readonly defaults: Settings['default']
  /** The base-file options (the mock-up's static select options). */
  readonly baseFiles: readonly string[]
}>()

const { t } = useI18n()

const baseFileFieldId = useId()
const titleId = useId()

/** The closed scheme vocabulary, in the mock-up's radio order. */
const schemes: readonly ProjectScheme[] = ['A', 'B', 'C']

/** Each scheme radio's wording key (shared with the project settings
 * modal — one wording, two screens). */
const schemeTextKey = {
  A: 'projects.settings.schemeA',
  B: 'projects.settings.schemeB',
  C: 'projects.settings.schemeC',
} satisfies Record<ProjectScheme, CatalogKey>

/** The base file's stem: the derived extra-file names follow it (D-20;
 * the demo's stemOf — a display derivation). */
const stem = computed(() => props.defaults.baseName.replace(/\.md$/, ''))

const extraLocalName = computed(() => `${stem.value}.local.md`)
const extraProjectName = computed(() => `${stem.value}.project.md`)

/** DEF-001: the extra-files block is visible only for scheme A. */
const isSchemeA = computed(() => props.defaults.scheme === 'A')
</script>

<template>
  <section class="ps-section" :aria-labelledby="titleId">
    <h3 :id="titleId" class="ps-section-title">{{ t('settings.defaultsTitle') }}</h3>

    <fieldset class="ps-field ps-scheme-set">
      <legend class="field-label">{{ t('projects.settings.schemeLabel') }}</legend>
      <div class="ps-schemes">
        <label v-for="s in schemes" :key="s" class="ps-scheme">
          <input type="radio" name="set-scheme" :value="s" :checked="props.defaults.scheme === s" />
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
          :selected="file === props.defaults.baseName"
        >
          {{ file }}
        </option>
      </select>
    </div>

    <!-- Scheme A only (DEF-001) -->
    <div v-if="isSchemeA" class="ps-field">
      <span class="field-label">{{ t('projects.settings.extraFilesLabel') }}</span>
      <div class="ps-extra-row">
        <label class="ps-check-row">
          <input type="checkbox" :checked="props.defaults.extraLocal" />
          {{ t('projects.settings.includeExtra') }} <code>{{ extraLocalName }}</code>
        </label>
        <select
          class="ps-input ps-select ps-pos-select"
          :disabled="!props.defaults.extraLocal"
          :aria-label="t('projects.settings.posLocalLabel')"
        >
          <option value="start" :selected="props.defaults.extraLocalPos === 'start'">
            {{ t('projects.settings.posStart') }}
          </option>
          <option value="end" :selected="props.defaults.extraLocalPos === 'end'">
            {{ t('projects.settings.posEnd') }}
          </option>
        </select>
      </div>
      <div class="ps-extra-row">
        <label class="ps-check-row">
          <input type="checkbox" :checked="props.defaults.extraProject" />
          {{ t('projects.settings.includeExtra') }} <code>{{ extraProjectName }}</code>
        </label>
        <select
          class="ps-input ps-select ps-pos-select"
          :disabled="!props.defaults.extraProject"
          :aria-label="t('projects.settings.posProjectLabel')"
        >
          <option value="start" :selected="props.defaults.extraProjectPos === 'start'">
            {{ t('projects.settings.posStart') }}
          </option>
          <option value="end" :selected="props.defaults.extraProjectPos === 'end'">
            {{ t('projects.settings.posEnd') }}
          </option>
        </select>
      </div>
    </div>

    <p class="ps-hint">{{ t('settings.defaultsHint') }}</p>
  </section>
</template>
