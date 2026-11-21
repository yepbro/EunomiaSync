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
import { onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ipc } from './ipc'

const { t, n } = useI18n()

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
  <main class="container">
    <h1>{{ t('app.title') }}</h1>
    <p v-if="version" class="version">{{ t('app.versionLine', { version }) }}</p>
    <p v-if="projectCount !== null" class="projects">
      {{ t('app.trackedProjects', { count: n(projectCount) }, projectCount) }}
    </p>

    <div class="row">
      <a href="https://vite.dev" target="_blank">
        <img src="/vite.svg" class="logo vite" :alt="t('app.logoAlt.vite')" />
      </a>
      <a href="https://tauri.app" target="_blank">
        <img src="/tauri.svg" class="logo tauri" :alt="t('app.logoAlt.tauri')" />
      </a>
      <a href="https://vuejs.org/" target="_blank">
        <img src="./assets/vue.svg" class="logo vue" :alt="t('app.logoAlt.vue')" />
      </a>
    </div>
    <p>{{ t('app.tagline') }}</p>
  </main>
</template>

<style scoped>
.version,
.projects {
  margin: 0 0 0.5em;
  opacity: 0.6;
}

.logo.vite:hover {
  filter: drop-shadow(0 0 2em #747bff);
}

.logo.vue:hover {
  filter: drop-shadow(0 0 2em #249b73);
}
</style>
<style>
:root {
  font-family: Inter, Avenir, Helvetica, Arial, sans-serif;
  font-size: 16px;
  line-height: 24px;
  font-weight: 400;

  color: #0f0f0f;
  background-color: #f6f6f6;

  font-synthesis: none;
  text-rendering: optimizeLegibility;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
  -webkit-text-size-adjust: 100%;
}

.container {
  margin: 0;
  padding-top: 10vh;
  display: flex;
  flex-direction: column;
  justify-content: center;
  text-align: center;
}

.logo {
  height: 6em;
  padding: 1.5em;
  will-change: filter;
  transition: 0.75s;
}

.logo.tauri:hover {
  filter: drop-shadow(0 0 2em #24c8db);
}

.row {
  display: flex;
  justify-content: center;
}

a {
  font-weight: 500;
  color: #646cff;
  text-decoration: inherit;
}

a:hover {
  color: #535bf2;
}

h1 {
  text-align: center;
}

@media (prefers-color-scheme: dark) {
  :root {
    color: #f6f6f6;
    background-color: #2f2f2f;
  }

  a:hover {
    color: #24c8db;
  }
}
</style>
