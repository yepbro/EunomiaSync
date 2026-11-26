/*
 * Seed backups (T-366): the Backups tab's demo rows (BKP-004). The
 * prototype builds them per open project from its syncedAt stamp
 * (docs/prototype/app.js renderBackups, ~3049 — three copies, the day
 * shifted back per row, sizes 8.4/8.1/7.9 KB, the run number counting
 * down) — JS is never ported, so the seed stores the builder's OUTPUT
 * as display strings, keyed by the project name for the settings
 * showcases. Each row also carries `date` (the day part the BKP-012
 * save tooltip names beside the file).
 */

import type { BackupRow } from './types'

/** The Backups rows of the projects the settings showcases open. */
export const backupsByProject: Readonly<Record<string, readonly BackupRow[]>> = {
  EunomiaSync: [
    {
      date: '19 Nov 2026',
      when: '19 Nov 2026, 14:32 (UTC+3)',
      file: 'AGENTS.md',
      size: '8.4 KB',
      run: '#47',
    },
    {
      date: '18 Nov 2026',
      when: '18 Nov 2026, 14:32 (UTC+3)',
      file: 'AGENTS.md',
      size: '8.1 KB',
      run: '#46',
    },
    {
      date: '17 Nov 2026',
      when: '17 Nov 2026, 14:32 (UTC+3)',
      file: 'AGENTS.md',
      size: '7.9 KB',
      run: '#45',
    },
  ],
  'yepbro/site': [
    {
      date: '19 Nov 2026',
      when: '19 Nov 2026, 09:14 (UTC+3)',
      file: 'AGENTS.md',
      size: '8.4 KB',
      run: '#12',
    },
    {
      date: '18 Nov 2026',
      when: '18 Nov 2026, 09:14 (UTC+3)',
      file: 'AGENTS.md',
      size: '8.1 KB',
      run: '#11',
    },
    {
      date: '17 Nov 2026',
      when: '17 Nov 2026, 09:14 (UTC+3)',
      file: 'AGENTS.md',
      size: '7.9 KB',
      run: '#10',
    },
  ],
  docktree: [
    {
      date: '17 Nov 2026',
      when: '17 Nov 2026, 11:02 (UTC+3)',
      file: 'AGENTS.md',
      size: '8.4 KB',
      run: '#22',
    },
    {
      date: '16 Nov 2026',
      when: '16 Nov 2026, 11:02 (UTC+3)',
      file: 'AGENTS.md',
      size: '8.1 KB',
      run: '#21',
    },
    {
      date: '15 Nov 2026',
      when: '15 Nov 2026, 11:02 (UTC+3)',
      file: 'AGENTS.md',
      size: '7.9 KB',
      run: '#20',
    },
  ],
}
