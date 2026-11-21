// `eunomia/no-raw-ui-text` — an in-repo ESLint rule (T-142, spec UI-020):
// no user-visible raw text in Vue templates. Every UI string goes through
// a vue-i18n key (`t('app.title')`); this rule is the gate that keeps a
// raw string from sneaking back into a template.
//
// Why a custom rule: the card suggested `vue/no-raw-text` from
// eslint-plugin-vue, but no such rule exists in the installed
// eslint-plugin-vue 10.x (verified against the package's rule list). The
// rule of that name lives in `@intlify/eslint-plugin-vue-i18n`, an extra
// dependency outside this task's allow-list (vue-i18n only) — so the rule
// is written here instead, registered in eslint.config.js under the
// `eunomia` plugin prefix, and pinned by `no-raw-ui-text.spec.ts`
// (RuleTester red/green) so it cannot silently stop firing.
//
// What counts as a violation:
// - a template text node containing any letter or digit (punctuation,
//   symbols and whitespace are formatting, not copy) — `v{{ version }}`
//   is as guilty as "Click here", which is why interpolating copy into a
//   raw prefix does not pass: use a message with a named placeholder;
// - a literal value of a user-facing attribute (`alt`, `title`,
//   `placeholder`, `aria-label`, `aria-valuetext`, …) — bound expressions
//   (`:alt="t('app.logoAlt.vite')"`) are the sanctioned form;
// - `value` on submit-style inputs (`<input type="submit" value="…">`),
//   where the browser renders the literal as the control's label — on
//   text inputs, options and everything else `value` is data, not copy;
// - an interpolation or bound expression that shows bare literal copy —
//   `{{ 'Save' }}`, `{{ 'Sa' + 've' }}`, `` {{ `Save` }} ``,
//   `{{ t('k') + ' — Beta' }}`, `:title="'Save'"`. Only `+`
//   concatenations, template literals and lone literals are walked, and
//   everything inside a `t`/`n`/`$t` call counts as translation data —
//   comparisons (`x === 'en'`) and filter calls never fire.
//
// Known limit (accepted): copy computed in `<script setup>` and rendered
// through `{{ label }}` is invisible to this template-only rule, and so
// is copy smuggled through non-copy expression shapes (method calls
// returning literals, ternary arms). The catalog-coverage test in
// `i18n.spec.ts` (every `t('…')` literal in `.vue` files must exist in
// the catalog) catches the typo half of that story, and a script-side
// scan needs a dedicated rule if real screens start doing it.
//
// Not covered (by design): anything outside `.vue` templates — fixture
// and mock-layer data (`src/ipc/**`), the generated `src/bindings.ts`,
// `index.html` (no Vue template), URLs, CSS classes and identifiers.
// That boundary is recorded in DECISIONS.md (T-142 entry).

import type { Rule } from 'eslint'

/** Attributes whose literal value a user sees (tooltip) or hears (SR). */
const USER_FACING_ATTRIBUTES: ReadonlySet<string> = new Set([
  'alt',
  'aria-brailledescription',
  'aria-braillelabel',
  'aria-description',
  'aria-label',
  'aria-placeholder',
  'aria-roledescription',
  'aria-valuetext',
  'label',
  'placeholder',
  'title',
])

/**
 * `value` is user-facing copy only on submit-style inputs, where the
 * browser renders it as the control's label (`UI-012` forms); on text
 * inputs, options and everything else it is form data, not copy.
 */
const SUBMIT_INPUT_TYPES: ReadonlySet<string> = new Set(['button', 'reset', 'submit'])

/** Options accepted by the rule (both optional). */
export type NoRawUiTextOptions = {
  /** Regex source: raw text matching it (after trim) is exempt. */
  readonly ignorePattern?: string
  /** Exact raw strings (after trim) that are exempt. */
  readonly ignoreText?: readonly string[]
}

/** Minimal structural types for the vue-eslint-parser template AST. */
type TemplateTextNode = {
  readonly type: 'VText'
  readonly value: string
}

/** A `VAttribute` node (non-directive arm) with its `VIdentifier` key. */
type TemplateAttribute = {
  readonly type: 'VAttribute'
  readonly directive: boolean
  readonly key: { readonly type: 'VIdentifier'; readonly name: string | { readonly name?: string } }
  readonly value?: { readonly type: 'VLiteral'; readonly value: string }
  readonly parent?: unknown
}

/** The `VStartTag` an attribute belongs to, with its element. */
type TemplateStartTag = {
  readonly type: 'VStartTag'
  // `name` is the parser's lowercased tag name; `rawName` keeps source
  // casing (`<INPUT>` → rawName "INPUT", name "input").
  readonly parent?: { readonly type: 'VElement'; readonly name?: string; readonly rawName?: string }
  readonly attributes?: readonly unknown[]
}

/** A `VExpressionContainer` (`{{ … }}`, or a directive's expression). */
type TemplateExpressionContainer = {
  readonly type: 'VExpressionContainer'
  readonly expression?: TemplateExpression | null
}

/** The structural shapes of a template expression the rule inspects. */
type TemplateExpression = {
  readonly type: string
  readonly name?: string
  readonly value?: unknown
  readonly operator?: string
  readonly left?: TemplateExpression
  readonly right?: TemplateExpression
  readonly callee?: TemplateExpression
  readonly quasis?: readonly { readonly value?: { readonly cooked?: string } }[]
  readonly expressions?: readonly TemplateExpression[]
}

/** The composer calls whose string arguments are keys/values, not copy. */
const TRANSLATION_CALLS: ReadonlySet<string> = new Set(['$t', 'n', 't'])

/**
 * The bare copy of an expression: literal text that the user would see
 * and that is NOT inside a translation call — `' — Beta'` in
 * `t('k') + ' — Beta'`, both halves of `'Sa' + 've'`, the quasis of
 * `` `Save` ``. Returns the concatenated copy, or `null` when the
 * expression shows no literal text outside `t`/`n` (a bare `{{ t('k') }}`,
 * a comparison like `x === 'en'`, a filter call, an identifier — all
 * sanctioned or out of scope).
 */
function bareExpressionCopy(node: TemplateExpression | null | undefined): string | null {
  if (node === null || node === undefined) return null
  if (node.type === 'Literal') {
    return typeof node.value === 'string' && node.value.trim() !== '' ? node.value : null
  }
  if (node.type === 'TemplateLiteral') {
    const copy = (node.quasis ?? [])
      .map((quasi) => quasi.value?.cooked ?? '')
      .join('')
      .trim()
    const fromExpressions = (node.expressions ?? [])
      .map((part) => bareExpressionCopy(part))
      .filter((part): part is string => part !== null)
      .join('')
    const whole = (copy + fromExpressions).trim()
    return whole === '' ? null : whole
  }
  if (node.type === 'BinaryExpression' && node.operator === '+') {
    const left = bareExpressionCopy(node.left)
    const right = bareExpressionCopy(node.right)
    if (left === null && right === null) return null
    return `${left ?? ''}${right ?? ''}`.trim()
  }
  if (node.type === 'CallExpression') {
    const callee = node.callee
    if (
      callee?.type === 'Identifier' &&
      callee.name !== undefined &&
      TRANSLATION_CALLS.has(callee.name)
    ) {
      return null
    }
  }
  return null
}

const rule: Rule.RuleModule = {
  meta: {
    type: 'suggestion',
    docs: {
      description:
        'disallow user-visible raw text in Vue templates — all UI strings go through vue-i18n keys (spec UI-020)',
    },
    schema: [
      {
        type: 'object',
        properties: {
          ignorePattern: { type: 'string' },
          ignoreText: { type: 'array', items: { type: 'string' } },
        },
        additionalProperties: false,
      },
    ],
    messages: {
      rawUiText:
        "Raw UI text '{{text}}' — render it through a vue-i18n key (t('…')) instead (spec UI-020).",
    },
  },

  create(context) {
    const options = (context.options[0] ?? {}) as NoRawUiTextOptions
    const ignorePattern =
      options.ignorePattern === undefined ? undefined : new RegExp(options.ignorePattern, 'u')
    const ignoreText = new Set(options.ignoreText ?? [])

    /** True when the trimmed text is human-readable copy, not formatting. */
    const isUserVisibleText = (raw: string): boolean => {
      const text = raw.trim()
      if (text === '' || ignoreText.has(text)) return false
      if (ignorePattern?.test(text)) return false
      // Any Unicode letter or digit makes the node copy; anything else
      // (punctuation, symbols, separators) is layout.
      return /\p{L}|\p{N}/u.test(text)
    }

    const report = (node: unknown, raw: string): void => {
      if (!isUserVisibleText(raw)) return
      context.report({
        node: node as Rule.Node,
        messageId: 'rawUiText',
        data: { text: raw.trim().slice(0, 60) },
      })
    }

    /** The attribute's static name, whichever spelling the AST uses. */
    const attributeName = (attribute: unknown): string | undefined => {
      if (attribute === null || typeof attribute !== 'object') return undefined
      const attr = attribute as TemplateAttribute
      if (attr.directive) return undefined
      const name = attr.key?.name
      return typeof name === 'string' ? name : name?.name
    }

    /** True for `value` on an `<input type="submit|button|reset">`. */
    const isSubmitInputValue = (attribute: unknown): boolean => {
      const attr = attribute as TemplateAttribute
      const tag = attr.parent as TemplateStartTag | undefined
      // `name` (lowercased) — `<INPUT type="submit">` must not slip by.
      if (tag?.parent?.name !== 'input') return false
      const typeAttribute = (tag.attributes ?? []).find(
        (candidate: unknown) => attributeName(candidate) === 'type',
      ) as TemplateAttribute | undefined
      const type = typeAttribute?.value?.value?.toLowerCase()
      return type !== undefined && SUBMIT_INPUT_TYPES.has(type)
    }

    // Only meaningful on .vue files parsed by vue-eslint-parser (the
    // parser eslint-plugin-vue's flat config already wires for them).
    const services = (
      context.sourceCode.parserServices as
        | {
            defineTemplateBodyVisitor?: (
              templateVisitor: Readonly<Record<string, (node: unknown) => void>>,
            ) => Rule.RuleListener
          }
        | undefined
    )?.defineTemplateBodyVisitor
    if (services === undefined) return {}
    return services({
      VText: (node: unknown) => {
        const textNode = node as TemplateTextNode
        report(node, textNode.value)
      },
      // Static attributes only (the selector's [directive=false]): a
      // bound `:alt="t('…')"` never lands here — it is the sanctioned form.
      'VAttribute[directive=false]': (node: unknown) => {
        const attr = node as TemplateAttribute
        const name = attributeName(node)
        if (name === undefined) return
        const userFacing =
          USER_FACING_ATTRIBUTES.has(name) || (name === 'value' && isSubmitInputValue(node))
        if (!userFacing) return
        if (attr.value?.type !== 'VLiteral') return
        report(attr.value, attr.value.value)
      },
      // An interpolation (or directive expression) that shows bare
      // literal copy — `{{ 'Save' }}`, `{{ 'Sa' + 've' }}`,
      // `` {{ `Save` }} ``, `{{ t('k') + ' — Beta' }}` — is copy that
      // skipped the catalog. The walk stays deliberately narrow: only
      // `+` concatenations, template literals and lone literals count,
      // and everything inside a `t`/`n`/`$t` call is translation data,
      // so comparisons (`x === 'en'`), filter calls and identifiers
      // never fire.
      VExpressionContainer: (node: unknown) => {
        const container = node as TemplateExpressionContainer
        const copy = bareExpressionCopy(container.expression)
        if (copy === null) return
        report(node, copy)
      },
    })
  },
}

export default rule
