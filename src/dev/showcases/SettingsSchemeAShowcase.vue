<script setup lang="ts">
// DEV showcase (T-366): the settings window on the scheme-A seed
// project (EunomiaSync — the extra-files block with its Include
// flags and position selects).
import SettingsSchemeShowcase from './SettingsSchemeShowcase.vue'

defineOptions({ inheritAttrs: false })
</script>

<template>
  <SettingsSchemeShowcase scheme="A" />
</template>
