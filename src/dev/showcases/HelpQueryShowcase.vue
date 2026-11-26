<script setup lang="ts">
// DEV showcase wrapper (T-367): the Help window with the demo query
// "sync" — the list narrows to the matching articles, the titles and
// body texts highlight, the "N of M articles match" counter shows.
import HelpShowcase from './HelpShowcase.vue'
</script>

<template>
  <HelpShowcase query="sync" />
</template>
