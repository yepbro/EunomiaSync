<script setup lang="ts">
// The DEV-only screen gallery (T-363): a small launcher that lists the
// ported screens (src/dev/screens.ts) and opens one through the
// `#screen=<id>` hash — read on mount and on every `hashchange`. The
// whole component exists only in DEV builds: App.vue mounts it through
// a DEV-guarded defineAsyncComponent, so the production bundle
// excludes it (and, with it, the only static path to src/dev/** — the
// build's dist/ grep is the standing proof). Its styles are a scoped
// block on purpose: Tailwind's source detection would otherwise emit
// this launcher's utility classes into the production CSS even with
// the component's JS excluded, and component styles ride the same
// dead import branch, so they stay out too.
//
// The shell entry's component is `null` (it is the running app):
// selecting it clears the hash. A screen WITH a component is mounted
// below the launcher and opened through `v-model:open` (the dialog
// contract of T-364…T-367); closing it from inside returns the hash to
// the shell. The registry holds the shell, the matrix family's five
// showcases and its two dialogs (T-364); the showcases are not dialogs
// and simply ignore the model binding.
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { screens } from './screens'

const { t } = useI18n()

/** The launcher's own panel visibility. */
const panelOpen = ref(false)

/** The id from `#screen=<id>`, or null when the hash names no screen. */
const activeId = ref<string | null>(null)

const readHash = (): void => {
  const match = /^#screen=(.+)$/.exec(window.location.hash)
  activeId.value = match?.[1] ?? null
}

/** True while the hash names a screen this registry knows. */
const activeScreen = computed(() => screens.find((screen) => screen.id === activeId.value) ?? null)

/** Open state forwarded to the mounted screen component (v-model:open). */
const screenOpen = ref(true)

watch(activeId, () => {
  screenOpen.value = true
})

// Closing the mounted screen from inside (its close event) returns to
// the shell: the hash clears, the component unmounts.
watch(screenOpen, (value) => {
  if (!value && activeScreen.value !== null) window.location.hash = ''
})

// The shell entry IS the running app: selecting it clears the hash
// (unmounting any open screen) instead of writing its own.
const openScreen = (screen: (typeof screens)[number]): void => {
  if (screen.component === null) {
    window.location.hash = ''
    return
  }
  window.location.hash = `#screen=${screen.id}`
}

onMounted(() => {
  readHash()
  window.addEventListener('hashchange', readHash)
})

onBeforeUnmount(() => {
  window.removeEventListener('hashchange', readHash)
})
</script>

<template>
  <div class="dev-gallery">
    <button
      type="button"
      class="btn btn-ghost"
      :aria-label="t('dev.gallery.launcherLabel')"
      @click="panelOpen = !panelOpen"
    >
      {{ t('dev.gallery.launcherLabel') }}
    </button>

    <div v-if="panelOpen" class="dev-gallery-panel">
      <p class="dev-gallery-title">{{ t('dev.gallery.panelTitle') }}</p>
      <button
        v-for="screen in screens"
        :key="screen.id"
        type="button"
        class="btn btn-ghost dev-gallery-entry"
        :aria-pressed="screen.id === activeId"
        :aria-label="t('dev.gallery.openScreen', { title: screen.title })"
        @click="openScreen(screen)"
      >
        {{ screen.title }}
        <span v-if="screen.id === activeId" aria-hidden="true">•</span>
      </button>
      <p class="dev-gallery-hint">{{ t('dev.gallery.hint') }}</p>
    </div>

    <!-- A registered screen with a component (a dialog): mounted open;
         the shell entry (component null) never lands here. -->
    <component
      :is="activeScreen.component"
      v-if="activeScreen !== null && activeScreen.component !== null"
      v-model:open="screenOpen"
    />
  </div>
</template>

<style scoped>
/* Dev-only styling for the launcher (kept off Tailwind utilities —
   see the script header); tokens come from theme.css / base.css. */
.dev-gallery {
  position: fixed;
  bottom: 12px;
  left: 12px;
  z-index: 50;
  font-size: 13px;
}

.dev-gallery-panel {
  margin-block-start: 8px;
  display: flex;
  width: 288px;
  flex-direction: column;
  align-items: stretch;
  gap: 4px;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: var(--surface);
  padding: 12px;
}

.dev-gallery-title {
  margin: 0;
  font-weight: 600;
}

.dev-gallery-entry {
  justify-content: flex-start;
}

.dev-gallery-hint {
  margin: 0;
  font-size: 11px;
  color: var(--ink-muted);
}
</style>
