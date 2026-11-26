<script setup lang="ts">
// The About window (T-367, UPD-005, REL-004): the mock-up's
// #about-dialog (docs/prototype/index.html 3844–3934) on the T-363
// dialog contract — BaseModal (size md — the reference's bare .modal
// width) + defineModel<boolean>('open'). The product facts render as
// definition rows from props (the brand line with the version chip,
// the identifier, the author, the dual license with its two links,
// the repository, the stable-channel line — D-27); the dependency
// licenses are the collapsible <details> (REL-004); the update block
// renders its at-rest state — the "Check for updates" button and the
// initial "Last check: never" hint in the aria-live area (the check
// itself, its result states and the install flow are the update
// card's, UPD-001…003). No behavior: the links are real anchors (the
// mock-up's hrefs), the button is inert.
import { useId } from 'vue'
import { useI18n } from 'vue-i18n'
import BaseModal from '../ui/BaseModal.vue'

defineProps<{
  /** The brand line's product name (the seed's fact). */
  readonly productName: string
  /** The version the brand chip and the row show. */
  readonly version: string
  /** The bundle identifier (REL-004). */
  readonly identifier: string
  /** The author row's value (REL-004). */
  readonly author: string
  /** The repository link (REL-004). */
  readonly repositoryUrl: string
  /** The repository link's visible label (the demo's scheme-less form). */
  readonly repositoryLabel: string
  /** The dependency-license demo list (REL-004). */
  readonly licenses: readonly string[]
}>()

const { t } = useI18n()

/** The dialog contract (T-363): v-model:open drives the BaseModal. */
const open = defineModel<boolean>('open', { default: false })

const titleId = useId()
</script>

<template>
  <BaseModal v-model:open="open" size="md" :labelled-by="titleId">
    <form method="dialog">
      <h2 :id="titleId">{{ t('about.title') }}</h2>

      <div class="about-brand">
        <span class="brand-mark about-brand-mark" aria-hidden="true"></span>
        <span>{{ productName }}</span>
        <span class="chip">{{ version }}</span>
      </div>

      <dl class="about-info">
        <div class="about-row">
          <dt>{{ t('about.versionKey') }}</dt>
          <dd>{{ version }}</dd>
        </div>
        <div class="about-row">
          <dt>{{ t('about.identifierKey') }}</dt>
          <dd>
            <code>{{ identifier }}</code>
          </dd>
        </div>
        <div class="about-row">
          <dt>{{ t('about.authorKey') }}</dt>
          <dd>{{ author }}</dd>
        </div>
        <div class="about-row">
          <dt>{{ t('about.licenseKey') }}</dt>
          <dd>
            <i18n-t keypath="about.licenseValue" tag="span" scope="global">
              <template #mit>
                <a
                  href="https://opensource.org/license/mit"
                  target="_blank"
                  rel="noopener noreferrer"
                  :title="t('about.licenseMitTitle')"
                >
                  {{ t('about.licenseMit') }}
                </a>
              </template>
              <template #apache>
                <a
                  href="https://www.apache.org/licenses/LICENSE-2.0"
                  target="_blank"
                  rel="noopener noreferrer"
                  :title="t('about.licenseApacheTitle')"
                >
                  {{ t('about.licenseApache') }}
                </a>
              </template>
            </i18n-t>
          </dd>
        </div>
        <div class="about-row">
          <dt>{{ t('about.repositoryKey') }}</dt>
          <dd>
            <a
              :href="repositoryUrl"
              target="_blank"
              rel="noopener noreferrer"
              :title="t('about.repositoryTitle')"
            >
              {{ repositoryLabel }}
            </a>
          </dd>
        </div>
        <div class="about-row">
          <dt>{{ t('about.channelKey') }}</dt>
          <dd>{{ t('about.channelValue') }}</dd>
        </div>
      </dl>

      <details class="about-licenses">
        <summary>{{ t('about.licensesSummary') }}</summary>
        <ul class="about-license-list">
          <li v-for="license in licenses" :key="license">{{ license }}</li>
        </ul>
        <p class="ps-hint">{{ t('about.licensesHint') }}</p>
      </details>

      <div class="about-update">
        <button type="button" class="btn btn-primary">{{ t('about.checkUpdates') }}</button>
        <!-- The update check's aria-live area at rest (UPD-001…003's
             other states are the update card's). -->
        <div class="about-check-result" aria-live="polite">
          <p class="ps-hint">{{ t('about.checkNever') }}</p>
        </div>
      </div>

      <div class="modal-actions">
        <button value="close" class="btn btn-ghost">{{ t('about.close') }}</button>
      </div>
    </form>
  </BaseModal>
</template>
