/*
 * Seed file-preview sample (T-366, MAIN-033): the static typed blocks
 * the File preview dialog renders — the next sync's managed content
 * for the seed project named below. The SPEC's contract is the build
 * text (§9.2), not the demo's rendering (D-203 (5) — the demo added
 * each instruction's name as a `##` heading and omitted the library's
 * section headings; that reading is not followed). Built by hand from
 * the seed library and assignments by ALG-020/ALG-022:
 *
 *  - a heading is included when its section holds at least one
 *    assigned instruction, at the library heading's level (depth 0 →
 *    h2, depth 1 → h3, §9.2's repeat('#', L));
 *  - instruction NAMES never appear (ALG-022) — only the effective
 *    text of each assigned cell (the "Bug-driven development" cell
 *    carries the "Bug-first" variant's text);
 *  - the empty instruction ("Pre-commit checks") contributes no block
 *    (§9.2: an empty text is not appended);
 *  - "Naming conventions" is NOT assigned to this project in the seed
 *    matrix (a changed-none cell), so neither it nor any block of it
 *    appears.
 *
 * The real dialog renders `project_build_preview`'s answer through
 * `markdown_preview` (7.10, SEC-005) — a behavior card's; this sample
 * rides the seed's DEV-only bundle gate like the rest of src/dev/.
 */

import type { PreviewBlock } from './types'

/** The project the sample is built for (the seed's scheme-A project). */
export const filePreviewProjectName = 'EunomiaSync'

/** The static managed-content sample (typed blocks, in file order). */
export const filePreviewBlocks: readonly PreviewBlock[] = [
  { kind: 'heading', level: 2, text: 'General' },
  { kind: 'heading', level: 3, text: 'Communication' },
  {
    kind: 'paragraph',
    text: "Always answer in the user's language; default is English.",
  },
  { kind: 'heading', level: 3, text: 'Code comments' },
  { kind: 'paragraph', text: 'Comments explain why, not what.' },
  { kind: 'heading', level: 2, text: 'Code Style' },
  {
    kind: 'paragraph',
    text: 'The formatter is the source of truth; never argue with it.',
  },
  { kind: 'paragraph', text: 'Full annotations on all public signatures.' },
  { kind: 'heading', level: 2, text: 'Git Workflow' },
  {
    kind: 'paragraph',
    text: 'Imperative mood, subject up to 72 characters; the body explains why.',
  },
  { kind: 'paragraph', text: 'feature/t-<n>-<slug> — from the task card number.' },
  { kind: 'heading', level: 2, text: 'Testing' },
  { kind: 'paragraph', text: 'Test behavior, not implementation.' },
  { kind: 'paragraph', text: 'Smoke scenarios run on every pull request.' },
  {
    kind: 'paragraph',
    text: 'Reproduce the bug in a failing test committed before the fix.',
  },
  { kind: 'heading', level: 2, text: 'Documentation' },
  { kind: 'paragraph', text: 'Every solved problem gets a D-NNN entry.' },
  { kind: 'heading', level: 2, text: 'Security' },
  {
    kind: 'paragraph',
    text: 'Never commit secrets; use environment variables and secret stores.',
  },
  { kind: 'paragraph', text: 'cargo-deny and npm audit run on CI.' },
  { kind: 'heading', level: 2, text: 'Performance' },
  { kind: 'paragraph', text: 'Initial bundle is at most 200 KB gzip.' },
]
