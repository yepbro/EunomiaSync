/*
 * Seed assignments (T-363): the matrix cells of the frozen prototype
 * (docs/prototype/index.html, the tbody `.assign` inputs and
 * `.variant-btn` buttons), keyed by instruction name then project
 * name. An absent pair means "not assigned" (the mock-up's empty
 * square) — the sparse table below carries only the non-default cells
 * and the module fills the defaults.
 *
 * `dirty: true` marks the demo's eight "(changed)" cells — the cells
 * whose current state differs from the demo's `data-initial` attr
 * (its value is the checkbox's initial checked state): three were
 * assigned and are now not ("not assigned (changed)"), five were not
 * and are now assigned ("main variant (changed)"). Changes made in the
 * app are what the next sync writes (the mock-up's dirty marker).
 * Disabled cells are NOT part of the data: a project's cells render
 * disabled when its status is `unavailable` (only utils/deja-vu in
 * the demo), which the rendering card derives.
 */

import type { Assignment, AssignmentMatrix } from './types'
import { library } from './library'
import { projects } from './projects'

const main: Assignment = { state: 'main', dirty: false }
const changedMain: Assignment = { state: 'main', dirty: true }
const changedNone: Assignment = { state: 'none', dirty: true }
const variant = (name: string): Assignment => ({
  state: 'variant',
  variant: name,
  dirty: false,
})

/** The non-default cells only; everything else defaults to not assigned. */
const sparseCells: Readonly<Record<string, Readonly<Record<string, Assignment>>>> = {
  'Language of responses': {
    EunomiaSync: main,
    'yepbro/site': variant('Spanish'),
    quotameter: main,
    'task-id-mcp': changedNone,
    docktree: main,
    'sandbox-cf': main,
  },
  'Response tone': { 'yepbro/site': main, docktree: changedMain },
  'Comment style': {
    EunomiaSync: main,
    'task-id-mcp': changedMain,
    'sandbox-cf': main,
  },
  'When to comment': { quotameter: main },
  'Formatting and linting': {
    EunomiaSync: main,
    'yepbro/site': main,
    'task-id-mcp': main,
    moli: main,
  },
  'Naming conventions': {
    EunomiaSync: changedNone,
    'task-id-mcp': variant('Strict'),
    docktree: main,
  },
  'Type hints': { EunomiaSync: main, quotameter: main, docktree: changedNone },
  'Line length': { 'yepbro/site': main },
  'Docstring format': { 'yepbro/site': main },
  'Commit messages': {
    EunomiaSync: main,
    'yepbro/site': main,
    quotameter: variant('Conventional Commits'),
    'task-id-mcp': main,
    docktree: variant('Conventional Commits'),
    moli: main,
  },
  'Branch naming': { EunomiaSync: main, 'task-id-mcp': main },
  // The demo's "empty variant assigned" cell: a main assignment of an
  // empty instruction (MAIN-009, D-13) — `main` of a text-less row.
  'Pre-commit checks': { EunomiaSync: main },
  'Unit tests': {
    EunomiaSync: main,
    'yepbro/site': main,
    'task-id-mcp': main,
    docktree: main,
    moli: changedMain,
  },
  'E2E tests': { EunomiaSync: main, 'task-id-mcp': main },
  'Coverage targets': { moli: changedMain },
  'Bug-driven development': { EunomiaSync: variant('Bug-first'), docktree: main },
  'README updates': { 'yepbro/site': main, docktree: main },
  'DECISIONS.md entries': { EunomiaSync: main, quotameter: main },
  'Secrets handling': {
    EunomiaSync: main,
    'yepbro/site': main,
    quotameter: main,
    'task-id-mcp': main,
    docktree: main,
    moli: main,
    'sandbox-cf': main,
  },
  'Dependency audit': { EunomiaSync: main, 'task-id-mcp': main },
  'Bundle size budget': { EunomiaSync: changedMain },
}

const notAssigned: Assignment = { state: 'none', dirty: false }

/**
 * The full matrix: every instruction × project pair of the seed, with
 * `{ state: 'none', dirty: false }` where the sparse table is silent.
 */
export const assignments: AssignmentMatrix = Object.fromEntries(
  library.flatMap((row) =>
    row.kind === 'instruction'
      ? [
          [
            row.name,
            Object.fromEntries(
              projects.map((project) => [
                project.name,
                sparseCells[row.name]?.[project.name] ?? notAssigned,
              ]),
            ),
          ],
        ]
      : [],
  ),
)
