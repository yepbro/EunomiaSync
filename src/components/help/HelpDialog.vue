<script setup lang="ts">
// The Help window (T-367, HLP-001…005): the mock-up's #help-dialog
// (docs/prototype/index.html 3792–3830) on the T-363 dialog contract —
// BaseModal (size xl) + defineModel<boolean>('open'). Two panes in the
// instruction-editor idiom: the article list and the search on the
// left, the selected article on the right (HelpArticleBody renders its
// typed blocks). The search input renders; the query state comes from
// the optional `query` prop (no search behavior — the help card's):
// when non-empty, the articles matching on title or body (HLP-001's
// match rule) stay listed with the title matches <mark>-wrapped, the
// "N of M articles match" counter shows (HLP-003), and at zero
// matches the list shows the empty state while the open article stays
// (HLP-001, UI-012). The selected article is this component's local
// state (click switching with the demo's aria-pressed idiom; when the
// selection falls out of the match set, the first match shows — the
// demo's fallback). The search resets on open and the selection
// survives reopening — behavior cards'.
import { computed, ref, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import BaseModal from '../ui/BaseModal.vue'
import HelpArticleBody from './HelpArticleBody.vue'
import { containsQuery, splitMark } from './highlight'
import type { HelpArticle, HelpBlock } from '../../dev/seed'

const props = withDefaults(
  defineProps<{
    /** The articles (the seed's demo corpus). */
    readonly articles: readonly HelpArticle[]
    /** The search query ('' is the at-rest window). */
    readonly query?: string
  }>(),
  { query: '' },
)

const { t } = useI18n()

/** The dialog contract (T-363): v-model:open drives the BaseModal. */
const open = defineModel<boolean>('open', { default: false })

const titleId = useId()
const listLabelId = useId()
const searchInputId = useId()
const searchHintId = useId()

/** The selected article's index; the at-rest window opens on the
 * demo's default (the first article). */
const selected = ref(0)

/** The article's searchable plain texts (the demo renders the body to
 * plain text for the corpus; the typed blocks carry the same words). */
const blockTexts = (block: HelpBlock): readonly string[] => {
  switch (block.kind) {
    case 'heading':
    case 'paragraph':
    case 'code':
      return [block.text]
    case 'list':
      return block.items
  }
}

/** The articles matching the query on title or body (HLP-001: the
 * search matches the title and the rendered body text). */
const matches = computed(() =>
  props.articles
    .map((article, index) => ({ article, index }))
    .filter(
      ({ article }) =>
        containsQuery(article.title, props.query) ||
        article.body.some((block) =>
          blockTexts(block).some((text) => containsQuery(text, props.query)),
        ),
    ),
)

/** The matching indexes (the demo keeps non-matching items in the DOM
 * and hides them). */
const matchIndexes = computed(() => new Set(matches.value.map((entry) => entry.index)))

/** The article actually shown: when the selection falls out of the
 * match set, the first match shows (the demo's fallback rule); with
 * zero matches the open article stays (HLP-001). */
const shown = computed(() => {
  if (matchIndexes.value.has(selected.value) || matchIndexes.value.size === 0) {
    return props.articles[selected.value]
  }
  const first = matches.value[0]?.index ?? selected.value
  return props.articles[first]
})

const shownIndex = computed(() => props.articles.indexOf(shown.value))

const hasQuery = computed(() => props.query.trim() !== '')

/** The counter's wording (HLP-003): "All N articles" at rest, "N of M
 * articles match" under a query (the demo's helpSearchHint facts). */
const counter = computed(() =>
  hasQuery.value
    ? t('help.counterMatch', { n: matches.value.length, m: props.articles.length })
    : t('help.counterAll', { n: props.articles.length }),
)
</script>

<template>
  <BaseModal v-model:open="open" size="xl" :labelled-by="titleId">
    <form method="dialog">
      <h2 :id="titleId">{{ t('help.title') }}</h2>

      <div class="help-body">
        <!-- Left pane: article list + search -->
        <aside class="help-nav" :aria-label="t('help.navLabel')">
          <span :id="listLabelId" class="field-label">{{ t('help.listLabel') }}</span>
          <div class="help-list" :aria-labelledby="listLabelId" :hidden="matches.length === 0">
            <button
              v-for="(article, index) in props.articles"
              :key="article.title"
              type="button"
              class="help-item"
              :hidden="!matchIndexes.has(index)"
              :aria-pressed="index === shownIndex"
              @click="selected = index"
            >
              <span class="help-item-label">
                <template v-for="(segment, i) in splitMark(article.title, props.query)" :key="i">
                  <mark v-if="segment.mark">{{ segment.text }}</mark>
                  <template v-else>{{ segment.text }}</template>
                </template>
              </span>
            </button>
          </div>
          <p class="hist-empty" :hidden="matches.length !== 0">{{ t('help.empty') }}</p>
          <div class="help-search">
            <label class="field-label" :for="searchInputId">{{ t('help.searchLabel') }}</label>
            <input
              :id="searchInputId"
              type="search"
              :placeholder="t('help.searchPlaceholder')"
              autocomplete="off"
              :value="props.query"
              :aria-describedby="searchHintId"
            />
            <p :id="searchHintId" class="help-hint" aria-live="polite">{{ counter }}</p>
          </div>
        </aside>

        <!-- Right pane: the selected article, title + typed body -->
        <div class="help-article-pane">
          <h3>{{ shown.title }}</h3>
          <div class="help-article">
            <HelpArticleBody :blocks="shown.body" :query="props.query" />
          </div>
        </div>
      </div>

      <div class="modal-actions">
        <button value="close" class="btn btn-ghost">{{ t('help.close') }}</button>
      </div>
    </form>
  </BaseModal>
</template>
