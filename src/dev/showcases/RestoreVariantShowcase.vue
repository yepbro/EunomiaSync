<script setup lang="ts">
// DEV showcase (T-365): the restore-variant confirmation (LIB-027)
// naming the seed history's deleted alternative — "Old wording", the
// variant whose delete row carries the panel's Restore button.
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import RestoreVariantDialog from '../../components/library/RestoreVariantDialog.vue'

defineOptions({ inheritAttrs: false })

const { t } = useI18n()

const open = ref(true)
</script>

<template>
  <section class="showcase">
    <h2 class="showcase-title">{{ t('dev.showcase.restoreVariant') }}</h2>
    <p class="showcase-hint">{{ t('dev.showcase.restoreHint') }}</p>
    <RestoreVariantDialog v-model:open="open" variant-name="Old wording" />
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
