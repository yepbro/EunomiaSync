<script setup lang="ts">
// DEV showcase (T-366): the scan window (PRJ-002…007, NFR-015) at the
// seed's demo walk end state — counters, current path, the active
// configuration from the settings seed, and the three result groups
// (New checked by default, Already added muted, Skipped with the
// reason badges and fix hints).
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import ScanDialog from '../../components/projects/ScanDialog.vue'
import {
  scanCurrentPath,
  scanFoldersChecked,
  scanKnown,
  scanNew,
  scanReposFound,
  scanRoot,
  scanRootPath,
  scanSkipped,
  settings,
} from '../seed'

defineOptions({ inheritAttrs: false })

const { t } = useI18n()

const open = ref(true)
</script>

<template>
  <section class="showcase">
    <h2 class="showcase-title">{{ t('dev.showcase.projectsScan') }}</h2>
    <p class="showcase-hint">{{ t('dev.showcase.projectsScanHint') }}</p>
    <ScanDialog
      v-model:open="open"
      :root="scanRoot"
      :root-path="scanRootPath"
      :max-depth="settings.scan.maxDepth"
      :exclude-names="settings.scan.excludeNames"
      :folders-checked="scanFoldersChecked"
      :repos-found="scanReposFound"
      :current-path="scanCurrentPath"
      :new-rows="scanNew"
      :known-rows="scanKnown"
      :skipped-rows="scanSkipped"
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
