/*
 * Seed preview sample (T-365): the static typed blocks the editor's
 * Preview tab renders (UI-003's "Preview" tab). The real tab renders
 * markdown_preview's sanitized output (7.10, SEC-005) — a behavior
 * card's; this skeleton renders the blocks as template markup (no
 * v-html), so the panel shows the markdown styles (.ie-preview) with
 * the same block kinds the renderer produces: headings, paragraphs, a
 * list and a fenced code block. The content rides the instruction the
 * showcases open ("Commit messages").
 */

import type { PreviewBlock } from './types'

/** The static Preview sample (typed blocks, in document order). */
export const previewSample: readonly PreviewBlock[] = [
  { kind: 'heading', level: 2, text: 'Commit messages' },
  {
    kind: 'paragraph',
    text: 'Imperative mood, subject up to 72 characters; the body explains why.',
  },
  {
    kind: 'list',
    items: [
      'feat — a new feature',
      'fix — a bug fix',
      'docs — documentation only changes',
      'refactor — neither fixes a bug nor adds a feature',
    ],
  },
  { kind: 'code', text: 'feat(matrix): add the column actions menu' },
  {
    kind: 'paragraph',
    text: 'The body explains why the change was made, not what changed.',
  },
]
