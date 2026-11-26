<script setup lang="ts">
// The Settings window's "Updates and About" section (T-367, DEF-001,
// D-10): the mock-up's fifth set-body section (docs/prototype/index.html
// 3569–3601) — the check-at-startup checkbox with its UPD-002 hint and
// the four link buttons: "Archived projects…" and "Excluded paths…"
// open the D-10 screens (DEF-001's link buttons), "About…" opens the
// About window (UPD-005), "Check for updates" opens it and runs the
// check (UPD-001). No behavior: the buttons are the mock-up's inert
// controls — the openings are the settings card's.
import { useId } from 'vue'
import { useI18n } from 'vue-i18n'

defineProps<{
  /** The stored check-at-startup flag (the seed's update.checkOnStart). */
  readonly checkOnStart: boolean
}>()

const { t } = useI18n()

const titleId = useId()
</script>

<template>
  <section class="ps-section" :aria-labelledby="titleId">
    <h3 :id="titleId" class="ps-section-title">{{ t('settings.updatesTitle') }}</h3>

    <div class="ps-field">
      <label class="ps-check-row">
        <input type="checkbox" :checked="checkOnStart" />
        {{ t('settings.checkOnStart') }}
      </label>
      <p class="ps-hint">{{ t('settings.updatesHint') }}</p>
      <div class="set-buttons">
        <button type="button" class="btn btn-ghost">{{ t('settings.openArchived') }}</button>
        <button type="button" class="btn btn-ghost">{{ t('settings.openExclusions') }}</button>
        <button type="button" class="btn btn-ghost">{{ t('settings.openAbout') }}</button>
        <button type="button" class="btn btn-ghost">{{ t('settings.checkUpdates') }}</button>
      </div>
    </div>
  </section>
</template>
