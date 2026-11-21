<script setup lang="ts">
// The reusable AppError alert (T-143, spec UI-011/NFR-018, §7.10): a
// pure presentational component that renders one wire `AppError` and
// answers UI-011's four questions —
//
//   - **What happened** — the localized `message_key` text, with the
//     error's `params` interpolated by vue-i18n (UI-020/NFR-006: the
//     backend never composes prose).
//   - **Where** — the `path`/`project` params on a labeled row (the
//     "where" facts §10 names). Param shapes beyond path/project are
//     not yet stable across producers, so the MVP catalog messages are
//     placeholder-free and the row is the one surface for them; every
//     param is always in the copyable envelope below.
//   - **Why** — the §10 code plus the OS facts (BKP-005: "OS code or
//     error type" stays visible, per D-034): `details.os_code` and
//     `details.os_kind` join the visible row when present;
//     `details.os_message` is raw diagnostic material and stays inside
//     the collapsed envelope (the bindings' documented contract).
//   - **What to do** — the §10 user action for the code, grouped in
//     `./app-error-actions.ts` (keyed by the closed ErrorCode union;
//     message-family keys ride their code's group, with the one
//     sanctioned family-key override — UPD-004's rolled-back
//     migration). The action message may interpolate `params` (the
//     override names the pre-migration backup path).
//
// Fallback policy for an unknown `message_key` (a family key the catalog
// does not carry yet, or a producer/registry drift): the title falls
// back to `errors.fallbackTitle` ("Something went wrong") — checked
// with `te()`, so the raw key string never leaks into the UI (vue-i18n
// would otherwise render it). The code, action, where row and envelope
// do not depend on the key and keep rendering.
//
// Technical details: the full §7.10 envelope (code, message_key,
// params, details) sits in a native `<details>` disclosure — collapsed
// by default, keyboard-operable for free — with a Copy button that
// puts the envelope JSON on the clipboard (`navigator.clipboard`, with
// the `execCommand` textarea fallback for webviews/contexts without the
// async clipboard API; no clipboard plugin dependency).
//
// Accessibility: the message cluster (context, title, where/why/action
// rows) sits in a `role="alert"` assertive live region — mounting the
// alert announces the error — while the collapsed disclosure and the
// copy feedback live outside it (a live region nested in another
// re-announces unpredictably). The copy outcome is announced by a
// visually hidden `role="status"` region and is never color-only: the
// button label flips to "Copied" or "Copy failed", reverting after
// 2.5 s.
//
// The component is pure: `error` (+ the optional `contextKey`) in, no
// IPC, no store. Callers catch the rejected `ipc.commands.*` promise
// and pass the wire payload; wiring it into screens is the screens'
// tasks (the same tested-today/consumed-later shape as the T-013
// registry itself).
import { computed, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import type { AppErrorWire } from '../ipc'
import { errorActionKey } from './app-error-actions'

/**
 * The operation the caller was performing, as a catalog key (e.g.
 * `'project.add.title'`) — rendered as a muted lead-in line above the
 * title when the key exists in the catalog; unknown keys render
 * nothing (same discipline as the title fallback: no raw key strings
 * in the UI).
 */
const props = defineProps<{
  /** The §7.10 wire error to render. */
  error: AppErrorWire
  /** Optional i18n key naming the operation that failed. */
  contextKey?: string
}>()

const { t, te } = useI18n()

/** "What happened": the message_key text (params interpolated) or the honest fallback. */
const title = computed(() =>
  te(props.error.message_key)
    ? t(props.error.message_key, props.error.params)
    : t('errors.fallbackTitle'),
)

/** The caller's operation line, when a resolvable context key was given. */
const context = computed(() =>
  props.contextKey !== undefined && te(props.contextKey) ? t(props.contextKey) : null,
)

/** "Where": the path/project params, composed per what is present. */
const whereText = computed(() => {
  const { path, project } = props.error.params
  if (path !== undefined && project !== undefined)
    return t('errors.where.projectPath', { path, project })
  if (path !== undefined) return t('errors.where.path', { path })
  if (project !== undefined) return t('errors.where.project', { project })
  return null
})

/** "Why": the code plus the visible OS facts (os_code/os_kind, BKP-005). */
const whyText = computed(() => {
  const { code } = props.error
  const details = props.error.details
  const osCode = details?.os_code ?? null
  const osKind = details?.os_kind ?? null
  if (osCode !== null && osKind !== null)
    return t('errors.why.osCodeKind', { code, osCode: String(osCode), osKind })
  if (osCode !== null) return t('errors.why.osCode', { code, osCode: String(osCode) })
  if (osKind !== null) return t('errors.why.kind', { code, osKind })
  return t('errors.why.code', { code })
})

/** "What to do": the §10 user action group (or the family-key override). */
const actionText = computed(() =>
  t(errorActionKey(props.error.code, props.error.message_key), props.error.params),
)

/** The copyable §7.10 envelope, stable field order, 2-space JSON. */
const envelope = computed(() =>
  JSON.stringify(
    {
      code: props.error.code,
      message_key: props.error.message_key,
      params: props.error.params,
      details: props.error.details,
    },
    null,
    2,
  ),
)

// --- Copy -----------------------------------------------------------------

/** The copy button's feedback state; reverts to 'idle' after a delay. */
type CopyState = 'idle' | 'copied' | 'failed'
const copyState = ref<CopyState>('idle')
let copyRevertTimer: ReturnType<typeof setTimeout> | undefined

/** The pre-smartphone copy path: a hidden selected textarea + execCommand. */
function legacyCopy(text: string): boolean {
  if (typeof document.execCommand !== 'function') return false
  const focusedBefore = document.activeElement
  const area = document.createElement('textarea')
  area.value = text
  area.setAttribute('readonly', '')
  area.style.position = 'fixed'
  area.style.opacity = '0'
  document.body.appendChild(area)
  area.select()
  try {
    return document.execCommand('copy')
  } finally {
    // Never leak the textarea (execCommand may throw), and put focus
    // back where the user came from (select() moved it).
    area.remove()
    if (focusedBefore instanceof HTMLElement) focusedBefore.focus()
  }
}

/** Puts `text` on the clipboard; false when every path failed. */
async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator?.clipboard?.writeText !== undefined) {
      await navigator.clipboard.writeText(text)
      return true
    }
  } catch {
    // A webview may expose the API and still reject (permissions,
    // insecure context): fall through to the legacy path.
  }
  try {
    return legacyCopy(text)
  } catch {
    // execCommand may throw (a SecurityError in some webviews): a
    // failed copy must land in the 'failed' feedback state, not
    // escape the click handler as an unhandled rejection.
    return false
  }
}

async function copyEnvelope(): Promise<void> {
  copyState.value = (await copyText(envelope.value)) ? 'copied' : 'failed'
  clearTimeout(copyRevertTimer)
  copyRevertTimer = setTimeout(() => {
    copyState.value = 'idle'
  }, 2500)
}

onUnmounted(() => clearTimeout(copyRevertTimer))
</script>

<template>
  <div class="app-error">
    <!-- role=alert on the message cluster only: the disclosure and the
         copy feedback are tools, not part of the announced error, and a
         live region nested in another live region re-announces
         unpredictably. -->
    <div role="alert" class="message">
      <p v-if="context" class="context">{{ context }}</p>
      <p class="title">
        <span class="icon" aria-hidden="true">⚠</span>
        {{ title }}
      </p>
      <dl class="facts">
        <template v-if="whereText !== null">
          <dt>{{ t('errors.where.label') }}</dt>
          <dd>{{ whereText }}</dd>
        </template>
        <dt>{{ t('errors.why.label') }}</dt>
        <dd>
          <code>{{ whyText }}</code>
        </dd>
        <dt>{{ t('errors.actions.label') }}</dt>
        <dd>{{ actionText }}</dd>
      </dl>
    </div>
    <details class="envelope-details">
      <summary>{{ t('errors.details.summary') }}</summary>
      <pre class="envelope">{{ envelope }}</pre>
      <button type="button" class="copy" @click="copyEnvelope">
        {{
          copyState === 'copied'
            ? t('errors.details.copied')
            : copyState === 'failed'
              ? t('errors.details.copyFailed')
              : t('errors.details.copy')
        }}
      </button>
    </details>
    <span role="status" class="visually-hidden">
      {{
        copyState === 'copied'
          ? t('errors.details.copiedAnnouncement')
          : copyState === 'failed'
            ? t('errors.details.copyFailedAnnouncement')
            : ''
      }}
    </span>
  </div>
</template>

<style scoped>
.app-error {
  border: 1px solid #b91c1c;
  border-left-width: 4px;
  border-radius: 8px;
  background: #fef2f2;
  padding: 0.75rem 1rem;
  text-align: left;
  max-width: 60ch;
}

.context {
  margin: 0 0 0.25rem;
  opacity: 0.7;
}

.title {
  margin: 0 0 0.5rem;
  font-weight: 600;
}

.icon {
  margin-right: 0.25rem;
}

.facts {
  margin: 0 0 0.5rem;
  display: grid;
  grid-template-columns: auto 1fr;
  gap: 0.125rem 0.75rem;
}

.facts dt {
  opacity: 0.7;
}

.facts dd {
  margin: 0;
  overflow-wrap: anywhere;
}

.facts code {
  font-size: 0.9em;
}

.envelope-details summary {
  cursor: pointer;
}

.envelope {
  margin: 0.5rem 0;
  padding: 0.5rem;
  background: #f3f4f6;
  border-radius: 4px;
  font-size: 0.85rem;
  overflow-x: auto;
}

.copy {
  min-height: 24px;
  padding: 0.25rem 0.75rem;
  border: 1px solid #b91c1c;
  border-radius: 6px;
  background: transparent;
  color: inherit;
  cursor: pointer;
}

.copy:focus-visible {
  outline: 2px solid currentColor;
  outline-offset: 2px;
}

/* Screen-reader-only live region (announces the copy feedback). */
.visually-hidden {
  position: absolute;
  width: 1px;
  height: 1px;
  margin: -1px;
  padding: 0;
  border: 0;
  clip: rect(0 0 0 0);
  clip-path: inset(50%);
  overflow: hidden;
  white-space: nowrap;
}

@media (prefers-color-scheme: dark) {
  .app-error {
    border-color: #f87171;
    background: #2c1a1a;
  }

  .envelope {
    background: #1f1f1f;
  }

  .copy {
    border-color: #f87171;
  }
}
</style>
