/*
 * Seed help (T-367): the demo FAQ of the frozen prototype
 * (docs/prototype/app.js HELP ~4642), reduced to data. The demo stores
 * Markdown source and renders it with a mini renderer — JS is never
 * ported (D-206) — so the seed stores the bodies as TYPED BLOCKS
 * (./types.ts HelpBlock: heading, paragraph, code, ordered/unordered
 * list) the template renders directly. THIS card ports the full body
 * of two articles ("Quick start" and "Schemes A/B/C" — the most
 * structural ones: the ordered steps and the #### sections) and a
 * two-paragraph body for the rest; the complete articles are the
 * content cards' (T-216…T-218, HLP-005). The demo's inline Markdown
 * emphasis (backticks, bold) is not modeled — the texts carry its
 * words as plain strings.
 *
 * Article titles and bodies are CONTENT, not UI copy: they render
 * as-is and are the search's corpus.
 */

import type { HelpArticle } from './types'

/** The demo articles, in the list's order (the demo HELP array). */
export const helpArticles: readonly HelpArticle[] = [
  {
    title: 'Quick start',
    body: [
      {
        kind: 'paragraph',
        text: 'EunomiaSync stores instructions for AI agents in one local library and, on an explicit sync command, builds an AGENTS.md or CLAUDE.md file in each selected Git project — you write a rule once and assign it to the projects that need it.',
      },
      {
        kind: 'list',
        ordered: true,
        items: [
          'Add a project — pick a folder with .git, or run "Scan folder" to find repositories automatically (PRJ-001…007).',
          'Create an instruction — "New instruction" opens the editor: a name, the text of the Main variant, Markdown preview on the "Preview" tab (LIB-002).',
          'Assign it — click a cell in the assignment matrix; "Assign to projects…" covers many projects at once.',
          'Run sync — the pinned bottom panel builds and writes the target files. Nothing on disk changes until you run it (G5, OUT-011).',
        ],
      },
      {
        kind: 'paragraph',
        text: 'A synced file ends with a <!-- Created by EunomiaSync … --> creation marker (D-23); a build identical to the file on disk is not rewritten (D-24).',
      },
    ],
  },
  {
    title: 'Projects and scanning',
    body: [
      {
        kind: 'paragraph',
        text: 'A project is a local folder that is the root of a Git repository. A folder without .git is rejected with E_NOT_A_REPO; picking a subfolder offers the repository root (PRJ-001…004, D-08).',
      },
      {
        kind: 'paragraph',
        text: 'Scan walks the chosen folder up to scan.max_depth (6 by default, range 1–20, D-04) and skips node_modules, vendor, .git, dist, build, target, .venv by default — the list is editable in Settings (D-05).',
      },
    ],
  },
  {
    title: 'Instructions and variants',
    body: [
      {
        kind: 'paragraph',
        text: 'The library is one ordered structure of instructions and headings shared by all projects. An instruction has a main name and one or more variants; exactly one variant is the main one.',
      },
      {
        kind: 'paragraph',
        text: 'The editor is a large two-pane modal: variants on the left, the text on the right, with "Edit" and "Preview" tabs (UI-003). Markdown warnings — for example an unclosed code fence — are shown but never block saving (D-15).',
      },
    ],
  },
  {
    title: 'Schemes A/B/C',
    body: [
      {
        kind: 'paragraph',
        text: 'Every project picks a write scheme and a target file (AGENTS.md or CLAUDE.md) in its settings; a new project copies the defaults (PRJ-012, DEF-005).',
      },
      { kind: 'heading', text: 'Scheme A — full overwrite' },
      {
        kind: 'paragraph',
        text: 'The target file is replaced entirely. Optional <STEM>.local.md and <STEM>.project.md reading lines can be included at the start or the end; local rules are read last and take priority (D-21).',
      },
      { kind: 'heading', text: 'Scheme B — marked area' },
      {
        kind: 'paragraph',
        text: 'The file must exist and contain exactly one pair of <!-- EUNOMIASYNC:BEGIN --> / <!-- EUNOMIASYNC:END --> markers, each on its own line (D-19). The app never creates them — use "Copy marker block" and paste the block by hand. Missing, repeated, or reversed markers give the skipped status with E_MARKERS_INVALID, and content outside the markers is kept byte for byte, including the BOM and line breaks.',
      },
      { kind: 'heading', text: 'Scheme C — separate file' },
      {
        kind: 'paragraph',
        text: 'The build goes to <STEM>.project.md in the project root, fully overwritten; the main file is never opened, and a ready snippet for it is shown in the project settings (D-20).',
      },
      {
        kind: 'paragraph',
        text: 'Each written file ends with a <!-- Created by EunomiaSync … --> creation marker (D-23); content identical to the file on disk is not rewritten (D-24).',
      },
    ],
  },
  {
    title: 'Sync and unsynced changes',
    body: [
      {
        kind: 'paragraph',
        text: 'Files change only on the explicit "Run sync" command, which covers all active projects (G5, OUT-011). Any edit is saved to the database right away; project files are not touched until the sync (MAIN-011).',
      },
      {
        kind: 'paragraph',
        text: 'The summary groups every project as "Written", "Unchanged", "Skipped", or "Failed", with the path and reason (OUT-009, MAIN-031). An unchanged build (D-24) counts as synced and creates no backup.',
      },
    ],
  },
  {
    title: 'Backups',
    body: [
      {
        kind: 'paragraph',
        text: 'Before any file is written, backups of the existing target files are created and checked for all projects of the run; an error in even one backup cancels writing everywhere (OUT-006, BKP-005).',
      },
      {
        kind: 'paragraph',
        text: 'Up to backup.retention copies (10 by default, range 1–200) are kept per "project and file" pair; the oldest extras are removed after the run (BKP-003).',
      },
    ],
  },
  {
    title: 'History and versions',
    body: [
      {
        kind: 'paragraph',
        text: 'The app records versions of instructions and variants. A version is created only on a real change of the snapshot — saving without edits adds nothing (LIB-026…027, D-16).',
      },
      {
        kind: 'paragraph',
        text: 'Up to history.max_revisions versions (200 by default, range 10–1000) are kept per entity; the oldest ones are deleted (D-16).',
      },
    ],
  },
  {
    title: 'Limits and common errors',
    body: [
      {
        kind: 'paragraph',
        text: 'Variant text: Markdown, up to 262,144 characters; a line equal to a scheme B marker is rejected (E_CONTENT_RESERVED). CRLF is changed to LF on save (§6.9).',
      },
      {
        kind: 'paragraph',
        text: 'Common sync errors (§10): E_MARKERS_INVALID — scheme B markers are missing, repeated, or in the wrong order; E_FILE_LOCKED — the target file is locked by another process; close the program and retry, the backup is kept.',
      },
    ],
  },
]
