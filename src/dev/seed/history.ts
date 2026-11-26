/*
 * Seed history (T-365): the demo revisions the history panel lists
 * (LIB-026, 027). The frozen prototype builds its demo revisions in JS
 * per clicked row (app.js buildDemoRevisions, ~2404–2468, over the
 * HIST_NOW_MS demo clock at ~2392); JS is never ported, so the builder's
 * OUTPUT for one instruction is stored here as data — the same rows the
 * demo renders for "Commit messages" (the seed row with the main text,
 * a lazily-named alternative and one named alternative: the richest
 * revision list the demo can produce), newest first, exactly as the
 * builder sorts them (the smaller the minutes-back offset, the more
 * recent the revision). Stamps are the builder's histStamp() output for
 * each offset — "D Mon YYYY, HH:MM (UTC+3)" on the demo's fixed clock
 * ("now" = 19 Nov 2026, 14:32 UTC+3, matching the sync bar).
 *
 * Facts kept from the builder: rev_no is unique per entity; the latest
 * snapshot of every entity matches the current state; a variant's first
 * snapshots carry "(first draft)"/"(reworded)" wordings; "Old wording"
 * is the deleted alternative whose delete row carries the Restore
 * variant button (LIB-027); the last alternative goes through a
 * hide + restore pair; "Commit messages" is not hidden, so the
 * instruction entity carries its create row only.
 */

import type { HistoryRevision } from './types'

/** The instruction whose history the seed carries. */
export const historyInstruction = 'Commit messages'

/** The demo revisions for `historyInstruction`, newest first. */
export const historyRevisions: readonly HistoryRevision[] = [
  {
    entityType: 'variant',
    entityName: 'Conventional Commits',
    op: 'restore',
    revNo: 4,
    when: '18 Nov 2026, 14:32 (UTC+3)',
    snapshot: {
      kind: 'variant',
      name: 'Conventional Commits',
      text: 'type(scope): subject — feat, fix, docs, refactor, test, chore.',
    },
    restored: false,
  },
  {
    entityType: 'variant',
    entityName: 'Conventional Commits',
    op: 'hide',
    revNo: 3,
    when: '18 Nov 2026, 13:32 (UTC+3)',
    snapshot: {
      kind: 'variant',
      name: 'Conventional Commits',
      text: 'type(scope): subject — feat, fix, docs, refactor, test, chore.',
    },
    restored: false,
  },
  {
    entityType: 'variant',
    entityName: 'Conventional Commits',
    op: 'update',
    revNo: 2,
    when: '17 Nov 2026, 13:32 (UTC+3)',
    snapshot: {
      kind: 'variant',
      name: 'Conventional Commits',
      text: 'type(scope): subject — feat, fix, docs, refactor, test, chore.',
    },
    restored: false,
  },
  {
    entityType: 'variant',
    entityName: 'Main',
    op: 'update',
    revNo: 2,
    when: '17 Nov 2026, 12:52 (UTC+3)',
    snapshot: {
      kind: 'variant',
      name: 'Main',
      text: 'Imperative mood, subject up to 72 characters; the body explains why.',
    },
    restored: false,
  },
  {
    entityType: 'variant',
    entityName: 'Variant 1',
    op: 'update',
    revNo: 3,
    when: '17 Nov 2026, 12:32 (UTC+3)',
    snapshot: {
      kind: 'variant',
      name: 'Variant 1',
      text: 'Imperative mood, subject up to 72 characters; the body explains why. Slightly reworded version of the main text.',
    },
    restored: false,
  },
  {
    entityType: 'variant',
    entityName: 'Conventional Commits',
    op: 'create',
    revNo: 1,
    when: '16 Nov 2026, 14:32 (UTC+3)',
    snapshot: {
      kind: 'variant',
      name: 'Conventional Commits',
      text: 'type(scope): subject — feat, fix, docs, refactor, test, chore. (first draft)',
    },
    restored: false,
  },
  {
    entityType: 'variant',
    entityName: 'Variant 1',
    op: 'update',
    revNo: 2,
    when: '16 Nov 2026, 13:12 (UTC+3)',
    snapshot: {
      kind: 'variant',
      name: 'Variant 1',
      text: 'Imperative mood, subject up to 72 characters; the body explains why. Slightly reworded version of the main text. (reworded)',
    },
    restored: false,
  },
  {
    entityType: 'variant',
    entityName: 'Old wording',
    op: 'delete',
    revNo: 2,
    when: '15 Nov 2026, 15:32 (UTC+3)',
    snapshot: {
      kind: 'variant',
      name: 'Old wording',
      text: 'Imperative mood, subject up to 72 characters; the body explains why. (superseded wording)',
    },
    restored: false,
  },
  {
    entityType: 'variant',
    entityName: 'Variant 1',
    op: 'create',
    revNo: 1,
    when: '15 Nov 2026, 13:52 (UTC+3)',
    snapshot: {
      kind: 'variant',
      name: 'Variant 1',
      text: 'Imperative mood, subject up to 72 characters; the body explains why. Slightly reworded version of the main text. (first draft)',
    },
    restored: false,
  },
  {
    entityType: 'variant',
    entityName: 'Old wording',
    op: 'create',
    revNo: 1,
    when: '13 Nov 2026, 11:52 (UTC+3)',
    snapshot: {
      kind: 'variant',
      name: 'Old wording',
      text: 'Imperative mood, subject up to 72 characters; the body explains why. (superseded wording)',
    },
    restored: false,
  },
  {
    entityType: 'variant',
    entityName: 'Main',
    op: 'create',
    revNo: 1,
    when: '13 Nov 2026, 10:42 (UTC+3)',
    snapshot: {
      kind: 'variant',
      name: 'Main',
      text: 'Imperative mood, subject up to 72 characters; the body explains why. (first draft)',
    },
    restored: false,
  },
  {
    entityType: 'instruction',
    entityName: 'Commit messages',
    op: 'create',
    revNo: 1,
    when: '13 Nov 2026, 09:32 (UTC+3)',
    snapshot: { kind: 'instruction', name: 'Commit messages', state: 'active' },
    restored: false,
  },
]
