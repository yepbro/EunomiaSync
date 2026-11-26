<script setup lang="ts">
// DEV showcase (T-366): the settings window on the scheme-B seed
// project (yepbro/site — the marker hint with the "Copy marker
// block" button).
import SettingsSchemeShowcase from './SettingsSchemeShowcase.vue'

defineOptions({ inheritAttrs: false })
</script>

<template>
  <SettingsSchemeShowcase scheme="B" />
</template>
