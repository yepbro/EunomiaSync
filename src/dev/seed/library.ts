/*
 * Seed library (T-363): the matrix rows of the frozen prototype
 * (docs/prototype/index.html, tbody — 24 instruction rows under 9
 * headings) with the alternative variants of docs/prototype/app.js
 * (`VARIANTS`, ~line 1482). One array in document order; T-364 renders
 * it into the matrix and T-365's editor opens it.
 *
 * The demo shows a variant-count chip per row: "2 variants" for the
 * six instructions with two alternatives in VARIANTS, "1 variant" for
 * rows where the demo lazily materializes `Variant 1` with the main
 * text (app.js does it on open — the seed stores it explicitly so the
 * chip is data), no chip for main-text-only rows. Empty instructions
 * (MAIN-009) carry no text and no chip.
 *
 * The two hidden rows (LIB-005…007) keep their assignments; the demo
 * marks their texts with a literal hint sentence.
 */

import type { LibraryRow, Variant } from './types'

const variant = (name: string, text: string): Variant => ({ name, text })

/** `Variant 1` with the main text — the demo's lazy fallback, seeded. */
const mainAsVariant = (text: string): Variant[] => [variant('Variant 1', text)]

/** The library rows in document order (headings + instructions). */
export const library: readonly LibraryRow[] = [
  { kind: 'heading', name: 'General', depth: 0 },
  { kind: 'heading', name: 'Communication', depth: 1 },
  {
    kind: 'instruction',
    name: 'Language of responses',
    depth: 2,
    text: "Always answer in the user's language; default is English.",
    hidden: false,
    variants: [
      variant('Spanish', 'Responde siempre en el idioma del usuario; por defecto, en español.'),
      variant('Mandarin', '始终以用户的语言回答；默认为中文。'),
    ],
  },
  {
    kind: 'instruction',
    name: 'Response tone',
    depth: 2,
    text: 'Direct and friendly; no filler phrases.',
    hidden: false,
    variants: [],
  },
  { kind: 'heading', name: 'Code comments', depth: 1 },
  {
    kind: 'instruction',
    name: 'Comment style',
    depth: 2,
    text: 'Comments explain why, not what.',
    hidden: false,
    variants: mainAsVariant('Comments explain why, not what.'),
  },
  {
    kind: 'instruction',
    name: 'When to comment',
    depth: 2,
    text: 'Public API, workarounds, non-obvious algorithms.',
    hidden: false,
    variants: [],
  },
  { kind: 'heading', name: 'Code Style', depth: 0 },
  {
    kind: 'instruction',
    name: 'Formatting and linting',
    depth: 1,
    text: 'The formatter is the source of truth; never argue with it.',
    hidden: false,
    variants: [
      variant('Variant 1', 'The formatter is the source of truth; never argue with it.'),
      variant('Per-project configs', "Respect the project's own formatter config when it exists."),
    ],
  },
  {
    kind: 'instruction',
    name: 'Naming conventions',
    depth: 1,
    text: 'Descriptive names; no single-letter identifiers outside loop counters.',
    hidden: false,
    variants: [
      variant(
        'Variant 1',
        'Descriptive names; no single-letter identifiers outside loop counters.',
      ),
      variant('Strict', 'No abbreviations at all; names are full words, even in loops.'),
    ],
  },
  {
    kind: 'instruction',
    name: 'Type hints',
    depth: 1,
    text: 'Full annotations on all public signatures.',
    hidden: false,
    variants: mainAsVariant('Full annotations on all public signatures.'),
  },
  {
    kind: 'instruction',
    name: 'Line length',
    depth: 1,
    text: 'Soft limit 100 characters, hard limit 120.',
    hidden: false,
    variants: [],
  },
  {
    kind: 'instruction',
    name: 'Docstring format',
    depth: 1,
    text: 'Hidden instruction — visible only with the Show hidden toggle on.',
    hidden: true,
    variants: mainAsVariant('Hidden instruction — visible only with the Show hidden toggle on.'),
  },
  { kind: 'heading', name: 'Git Workflow', depth: 0 },
  {
    kind: 'instruction',
    name: 'Commit messages',
    depth: 1,
    text: 'Imperative mood, subject up to 72 characters; the body explains why.',
    hidden: false,
    variants: [
      variant('Variant 1', 'Imperative mood, subject up to 72 characters; the body explains why.'),
      variant(
        'Conventional Commits',
        'type(scope): subject — feat, fix, docs, refactor, test, chore.',
      ),
    ],
  },
  {
    kind: 'instruction',
    name: 'Branch naming',
    depth: 1,
    text: 'feature/t-<n>-<slug> — from the task card number.',
    hidden: false,
    variants: mainAsVariant('feature/t-<n>-<slug> — from the task card number.'),
  },
  {
    kind: 'instruction',
    name: 'Pre-commit checks',
    depth: 1,
    text: null,
    hidden: false,
    variants: [],
  },
  {
    kind: 'instruction',
    name: 'PR review checklist',
    depth: 1,
    text: 'Hidden instruction — visible only with the Show hidden toggle on.',
    hidden: true,
    variants: [
      variant('Variant 1', 'Check behavior, tests, docs and DECISIONS entries before approving.'),
      variant('Lightweight', 'Review only the diff: behavior and tests.'),
    ],
  },
  { kind: 'heading', name: 'Testing', depth: 0 },
  {
    kind: 'instruction',
    name: 'Unit tests',
    depth: 1,
    text: 'Test behavior, not implementation.',
    hidden: false,
    variants: mainAsVariant('Test behavior, not implementation.'),
  },
  {
    kind: 'instruction',
    name: 'E2E tests',
    depth: 1,
    text: 'Smoke scenarios run on every pull request.',
    hidden: false,
    variants: mainAsVariant('Smoke scenarios run on every pull request.'),
  },
  {
    kind: 'instruction',
    name: 'Coverage targets',
    depth: 1,
    text: 'Critical paths are covered at 80% or more.',
    hidden: false,
    variants: [],
  },
  {
    kind: 'instruction',
    name: 'Bug-driven development',
    depth: 1,
    text: 'A red test first, then the fix that makes it green.',
    hidden: false,
    variants: [
      variant('Variant 1', 'A red test first, then the fix that makes it green.'),
      variant('Bug-first', 'Reproduce the bug in a failing test committed before the fix.'),
    ],
  },
  { kind: 'heading', name: 'Documentation', depth: 0 },
  {
    kind: 'instruction',
    name: 'README updates',
    depth: 1,
    text: 'README is updated with any user-visible behavior change.',
    hidden: false,
    variants: mainAsVariant('README is updated with any user-visible behavior change.'),
  },
  {
    kind: 'instruction',
    name: 'DECISIONS.md entries',
    depth: 1,
    text: 'Every solved problem gets a D-NNN entry.',
    hidden: false,
    variants: mainAsVariant('Every solved problem gets a D-NNN entry.'),
  },
  {
    kind: 'instruction',
    name: 'Changelog',
    depth: 1,
    text: null,
    hidden: false,
    variants: [],
  },
  { kind: 'heading', name: 'Security', depth: 0 },
  {
    kind: 'instruction',
    name: 'Secrets handling',
    depth: 1,
    text: 'Never commit secrets; use environment variables and secret stores.',
    hidden: false,
    variants: mainAsVariant('Never commit secrets; use environment variables and secret stores.'),
  },
  {
    kind: 'instruction',
    name: 'Dependency audit',
    depth: 1,
    text: 'cargo-deny and npm audit run on CI.',
    hidden: false,
    variants: mainAsVariant('cargo-deny and npm audit run on CI.'),
  },
  { kind: 'heading', name: 'Performance', depth: 0 },
  {
    kind: 'instruction',
    name: 'Bundle size budget',
    depth: 1,
    text: 'Initial bundle is at most 200 KB gzip.',
    hidden: false,
    variants: mainAsVariant('Initial bundle is at most 200 KB gzip.'),
  },
  {
    kind: 'instruction',
    name: 'Profiling',
    depth: 1,
    text: 'Measure before optimizing.',
    hidden: false,
    variants: [],
  },
]

/** The "Show hidden" toggle's counter: the concealed instruction rows. */
export const hiddenInstructionCount = library.filter(
  (row) => row.kind === 'instruction' && row.hidden,
).length
