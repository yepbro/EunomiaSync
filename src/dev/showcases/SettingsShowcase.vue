<script setup lang="ts">
// DEV showcase (T-367): the app Settings window (DEF-001…005) — the
// §6.5 seed values in the five sections, the Backups field in the
// demo's range-error state (aria-invalid, the inline error, the typed
// 300 shown while the stored 10 keeps its place), the exclusions as
// chips. Nothing applies — the settings card's.
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import AppSettingsDialog from '../../components/settings/AppSettingsDialog.vue'
import { settingNumberRanges, settings, settingsRangeError } from '../seed'

defineOptions({ inheritAttrs: false })

const { t } = useI18n()

/** The base-file options (the mock-up's static select options). */
const BASE_FILES: readonly string[] = ['AGENTS.md', 'CLAUDE.md']

const open = ref(true)
</script>

<template>
  <section class="showcase">
    <h2 class="showcase-title">{{ t('dev.showcase.settings') }}</h2>
    <p class="showcase-hint">{{ t('dev.showcase.settingsHint') }}</p>
    <AppSettingsDialog
      v-model:open="open"
      :values="settings"
      :ranges="settingNumberRanges"
      :range-error="settingsRangeError"
      :base-files="BASE_FILES"
    />
  </section>
</template>

<style scoped>
/* Dev-only wrapper styling (scoped, like the gallery launcher). */
.showcase {
  margin: 16px;
}

.showcase-title {
  font-size: 14px;
  margin: 0 0 8px;
}

.showcase-hint {
  color: var(--ink-muted);
  font-size: 11px;
  margin: 0 0 12px;
}
</style>
