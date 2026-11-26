<script setup lang="ts">
// DEV showcase (T-367): the About window (UPD-005, REL-004) — the
// product rows, the stable-channel line and the collapsible
// dependency licenses; the check button is inert (the update card's).
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import AboutDialog from '../../components/about/AboutDialog.vue'
import {
  appAuthor,
  appIdentifier,
  dependencyLicenses,
  productName,
  productVersion,
  repositoryLabel,
  repositoryUrl,
} from '../seed'

defineOptions({ inheritAttrs: false })

const { t } = useI18n()

const open = ref(true)
</script>

<template>
  <section class="showcase">
    <h2 class="showcase-title">{{ t('dev.showcase.about') }}</h2>
    <p class="showcase-hint">{{ t('dev.showcase.aboutHint') }}</p>
    <AboutDialog
      v-model:open="open"
      :product-name="productName"
      :version="productVersion"
      :identifier="appIdentifier"
      :author="appAuthor"
      :repository-url="repositoryUrl"
      :repository-label="repositoryLabel"
      :licenses="dependencyLicenses"
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
