<script setup lang="ts">
// All IPC goes through the `src/ipc` facade (D-003: the tauri-specta
// `commands` live in src/bindings.ts; no component calls invoke()
// directly). The mock backend — same facade, fixture data, no Rust — is
// one flag away: VITE_IPC_MOCK=1 (README → "Development without Rust").
import { onMounted, ref } from 'vue'
import { ipc } from './ipc'

// The About-window facts (§7.10 `app_info`); the version line proves the
// IPC chain end to end — facade → backend → typed wire DTO.
const version = ref<string | null>(null)

onMounted(async () => {
  try {
    version.value = (await ipc.commands.appInfo()).version
  } catch {
    // No backend answer (e.g. a plain browser without the mock flag):
    // keep the shell rendered rather than half-crashed.
    version.value = null
  }
})
</script>

<template>
  <main class="container">
    <h1>EunomiaSync</h1>
    <p v-if="version" class="version">v{{ version }}</p>

    <div class="row">
      <a href="https://vite.dev" target="_blank">
        <img src="/vite.svg" class="logo vite" alt="Vite logo" />
      </a>
      <a href="https://tauri.app" target="_blank">
        <img src="/tauri.svg" class="logo tauri" alt="Tauri logo" />
      </a>
      <a href="https://vuejs.org/" target="_blank">
        <img src="./assets/vue.svg" class="logo vue" alt="Vue logo" />
      </a>
    </div>
    <p>Click on the Tauri, Vite, and Vue logos to learn more.</p>
  </main>
</template>

<style scoped>
.version {
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
