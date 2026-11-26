/*
 * Seed projects (T-363): the demo project data of the frozen prototype
 * (docs/prototype/app.js `PROJECTS`, ~line 2792) and its palette
 * (`PS_PALETTE`, ~line 2774), typed by ./types.ts. The `status` field
 * is read from the mock-up's column-header sub-lines (unsynced changes
 * / modified externally / unavailable / target deleted / a date), not
 * from the demo objects — the demo hardcodes it in the markup.
 *
 * Seed texts (names, paths, descriptions, timestamps) are DATA, not UI
 * copy: they render as-is from the components that receive them.
 */

import type { Project } from './types'

/** The project color palette the settings dialog offers (demo PS_PALETTE). */
export const projectPalette: readonly (readonly [hex: string, name: string])[] = [
  ['#ef4444', 'Red'],
  ['#f97316', 'Orange'],
  ['#f59e0b', 'Amber'],
  ['#84cc16', 'Lime'],
  ['#10b981', 'Green'],
  ['#14b8a6', 'Teal'],
  ['#06b6d4', 'Cyan'],
  ['#3b82f6', 'Blue'],
  ['#6366f1', 'Indigo'],
  ['#8b5cf6', 'Violet'],
  ['#ec4899', 'Pink'],
  ['#64748b', 'Slate'],
]

/** The demo projects, in the mock-up's column order (app.js PROJECTS). */
export const projects: readonly Project[] = [
  {
    name: 'EunomiaSync',
    label: 'EUNOMI',
    color: '#10b981',
    description: 'The app itself, syncing these instructions into its own AGENTS.md.',
    path: '/home/brooks/projects/business/EunomiaSync',
    origin: 'manual',
    scheme: 'A',
    baseFile: 'AGENTS.md',
    extraLocal: true,
    extraLocalPos: 'end',
    extraProject: false,
    extraProjectPos: 'end',
    syncedAt: '19 Nov 2026, 14:32 (UTC+3)',
    runs: 47,
    status: 'dirty',
  },
  {
    name: 'yepbro/site',
    label: 'YEPBR',
    color: '#3b82f6',
    description: 'Marketing site repo; agent rules live in the marked area of AGENTS.md.',
    path: '/home/brooks/projects/yepbro/site',
    origin: 'scan',
    scheme: 'B',
    baseFile: 'AGENTS.md',
    extraLocal: false,
    extraLocalPos: 'end',
    extraProject: false,
    extraProjectPos: 'end',
    syncedAt: '19 Nov 2026, 09:14 (UTC+3)',
    runs: 12,
    status: 'ok',
  },
  {
    name: 'quotameter',
    label: 'QUOTA',
    color: '#8b5cf6',
    description: 'Metering dashboard; the target file is CLAUDE.md here.',
    path: '/home/brooks/projects/business/quotameter',
    origin: 'manual',
    scheme: 'A',
    baseFile: 'CLAUDE.md',
    extraLocal: false,
    extraLocalPos: 'end',
    extraProject: true,
    extraProjectPos: 'end',
    syncedAt: '12 Nov 2026, 09:10 (UTC+3)',
    runs: 8,
    status: 'external',
  },
  {
    name: 'task-id-mcp',
    label: 'TASK-',
    color: '#f59e0b',
    description: 'Task numbering MCP service (Docker, autostart).',
    path: '/home/brooks/projects/utils/task-id-mcp',
    origin: 'manual',
    scheme: 'A',
    baseFile: 'AGENTS.md',
    extraLocal: true,
    extraLocalPos: 'end',
    extraProject: true,
    extraProjectPos: 'start',
    syncedAt: '18 Nov 2026, 20:41 (UTC+3)',
    runs: 31,
    status: 'dirty',
  },
  {
    name: 'utils/deja-vu',
    label: 'UTILS',
    color: '#14b8a6',
    description: 'Past-session memory search; the folder is currently unavailable.',
    path: '/home/brooks/projects/utils/deja-vu',
    origin: 'scan',
    scheme: 'A',
    baseFile: 'AGENTS.md',
    extraLocal: false,
    extraLocalPos: 'end',
    extraProject: false,
    extraProjectPos: 'end',
    syncedAt: '3 Nov 2026, 21:05 (UTC+3)',
    runs: 5,
    status: 'unavailable',
  },
  {
    name: 'docktree',
    label: 'DOCKT',
    color: '#ec4899',
    description: 'Container layout helper; instructions go to a separate project file.',
    path: '/home/brooks/projects/tools/docktree',
    origin: 'manual',
    scheme: 'C',
    baseFile: 'AGENTS.md',
    extraLocal: false,
    extraLocalPos: 'end',
    extraProject: false,
    extraProjectPos: 'end',
    syncedAt: '17 Nov 2026, 11:02 (UTC+3)',
    runs: 22,
    status: 'dirty',
  },
  {
    name: 'moli',
    label: 'MOLI',
    color: '#6366f1',
    description: 'Small gallery app; local overrides are included at the start of the file.',
    path: '/home/brooks/projects/apps/moli',
    origin: 'manual',
    scheme: 'A',
    baseFile: 'AGENTS.md',
    extraLocal: true,
    extraLocalPos: 'start',
    extraProject: false,
    extraProjectPos: 'end',
    syncedAt: '15 Nov 2026, 18:27 (UTC+3)',
    runs: 14,
    status: 'dirty',
  },
  {
    name: 'sandbox-cf',
    label: 'SANDB',
    color: '#64748b',
    description: 'Cloudflare experiments sandbox; the target file is not created yet.',
    path: '/home/brooks/projects/labs/sandbox-cf',
    origin: 'scan',
    scheme: 'B',
    baseFile: 'AGENTS.md',
    extraLocal: false,
    extraLocalPos: 'end',
    extraProject: false,
    extraProjectPos: 'end',
    syncedAt: '10 Nov 2026, 08:55 (UTC+3)',
    runs: 3,
    status: 'deleted',
  },
]

/**
 * The sync panel's last-sync stamp: the mock-up footer's static value —
 * the newest per-project `syncedAt` in the demo data (EunomiaSync's).
 * Kept as an explicit fact rather than a derived max: the stamps are
 * display strings, not comparable dates.
 */
export const lastSuccessfulSync = '19 Nov 2026, 14:32 (UTC+3)'

/**
 * The sync panel's unsynced-changes counter, derived from the statuses
 * (the mock-up footer shows 4; the four `dirty` projects above are its
 * source of truth).
 */
export const dirtyProjectCount = projects.filter((p) => p.status === 'dirty').length
