<script setup lang="ts">
// All IPC goes through the `src/ipc` facade (D-003: the tauri-specta
// `commands` live in src/bindings.ts; no component calls invoke()
// directly). The mock backend — same facade, fixture data, no Rust — is
// one flag away: VITE_IPC_MOCK=1 (README → "Development without Rust").
//
// All user-visible strings come from the i18n catalog (`src/locales/`,
// UI-020): `t('app.title')` etc., with the pluralized project count as
// the working pipe-plural example (`app.trackedProjects`). The
// `eunomia/no-raw-ui-text` lint rule keeps raw copy out of templates.
// The UI-021 theme bridge: the shell wires the OS light/dark setting to
// <html data-theme> on boot and keeps it in step with live OS changes —
// the palette lives in src/assets/theme.css (T-145).
import { onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useOsTheme } from './assets/theme'
import { ipc } from './ipc'

const { t, n } = useI18n()

useOsTheme()

// The About-window facts (§7.10 `app_info`); the version line proves the
// IPC chain end to end — facade → backend → typed wire DTO.
const version = ref<string | null>(null)

// Active projects (§7.10 `projects_list`) — drives the pluralized count
// line. `null` while unanswered (or no backend): the line stays hidden.
const projectCount = ref<number | null>(null)

onMounted(async () => {
  try {
    version.value = (await ipc.commands.appInfo()).version
  } catch {
    // No backend answer (e.g. a plain browser without the mock flag):
    // keep the shell rendered rather than half-crashed.
    version.value = null
  }
  try {
    projectCount.value = (await ipc.commands.projectsList(false)).length
  } catch {
    projectCount.value = null
  }
})
</script>

<template>
  <main class="flex flex-col items-center justify-center bg-page px-6 py-12 text-center text-ink">
    <h1 class="text-3xl font-semibold tracking-tight">{{ t('app.title') }}</h1>
    <p v-if="version" class="version mt-2 text-ink-muted">
      {{ t('app.versionLine', { version }) }}
    </p>
    <p v-if="projectCount !== null" class="projects text-ink-muted">
      {{ t('app.trackedProjects', { count: n(projectCount) }, projectCount) }}
    </p>

    <div class="mt-6 flex items-center justify-center gap-6">
      <a href="https://vite.dev" target="_blank">
        <img src="/vite.svg" class="size-24 p-3" :alt="t('app.logoAlt.vite')" />
      </a>
      <a href="https://tauri.app" target="_blank">
        <img src="/tauri.svg" class="size-24 p-3" :alt="t('app.logoAlt.tauri')" />
      </a>
      <a href="https://vuejs.org/" target="_blank">
        <img src="./assets/vue.svg" class="size-24 p-3" :alt="t('app.logoAlt.vue')" />
      </a>
    </div>
    <p class="mt-6 max-w-prose text-ink-muted">{{ t('app.tagline') }}</p>
  </main>
</template>
