// Domain shapes the fixture generator produces (T-141).
//
// Projects reuse the generated wire type: `projects_list` (T-072/D-096)
// is a library/assignment-adjacent command that existed in
// `src/bindings.ts` when this file was written, so its DTO is the
// contract fixtures must match. Library items and assignments kept
// their own fixture shapes (mirroring the §6.2 DDL columns they come
// from): `library_get` (T-083/D-187) has since generated its wire
// types, and the mock maps these fixtures onto them in
// `mock/state.ts`/`mock/commands.ts` — the assignment fixtures stay
// schema-shaped for the Assignments command group that is still
// future work.

import type { ProjectColumnEntryWire } from '../contract'

/**
 * One project column on the wire — the exact shape `projects_list` returns
 * (T-072 + T-257 + T-262 fields: id, name, label, color, state, available,
 * unavailable_reason, dirty, external_changed, file_deleted,
 * unwatched_reason, last_synced_at).
 */
export type ProjectFixture = ProjectColumnEntryWire

/**
 * DB-042's default label: the first up to 6 **code points** of the trimmed
 * name, upper-cased FIRST (`straße` → `STRASS`, not the 7-code-point
 * `STRASSE` — the same pin `project_validators.rs` holds on the Rust side;
 * code points, not UTF-16 units, so astral characters stay whole).
 * The generator and the `project_add_manual` mock share this one rule.
 */
export function defaultLabel(name: string): string {
  return Array.from(name.trim().toUpperCase()).slice(0, 6).join('')
}

/** The §6.2 `library_items` CHECK vocabulary for `kind`. */
export type LibraryItemKind = 'instruction' | 'heading'

/** A section heading row (§6.2: heading_level 2–6, never hidden). */
export type HeadingFixture = {
  readonly kind: 'heading'
  readonly id: string
  /** Display order in the library (§6.2 `position`). */
  readonly position: number
  readonly headingLevel: 2 | 3 | 4 | 5 | 6
  readonly title: string
  readonly state: 'active'
}

/** One variant of an instruction (§6.2 `instruction_variants`). */
export type VariantFixture = {
  readonly id: string
  /** 1–100 chars, unique within the instruction (NOCASE); main is `Main`. */
  readonly name: string
  /** Markdown text up to the §6.9 ceiling. */
  readonly content: string
  readonly isMain: boolean
}

/** An instruction row with its variants (§6.2). */
export type InstructionFixture = {
  readonly kind: 'instruction'
  readonly id: string
  readonly position: number
  readonly title: string
  readonly state: 'active' | 'hidden'
  /** Exactly one `isMain` member while active (the §6.2 partial index). */
  readonly variants: readonly VariantFixture[]
}

export type LibraryItemFixture = HeadingFixture | InstructionFixture

/**
 * One assignment (§6.2 `project_instructions`): a project has an
 * instruction in exactly one variant.
 */
export type AssignmentFixture = {
  readonly projectId: string
  readonly instructionId: string
  readonly variantId: string
}

/**
 * One excluded-path row (§6.2 `project_exclusions`) for the D-10
 * "Excluded paths" screen: the identity `exclusion_remove` addresses,
 * the display path, its DB-030 identity key, and the §6.2 CHECK reason.
 */
export type ExclusionFixture = {
  readonly id: string
  readonly path: string
  readonly pathKey: string
  readonly reason: 'removed_scanned' | 'manual'
  readonly createdAt: string
}

/**
 * The DB-030 stand-in identity key: `/` separators, no trailing
 * separator. Shared by the fixture generator and the mock's
 * `project_add_manual` duplicate check, so rows and probes agree.
 */
export function pathKeyOf(path: string): string {
  const trimmed = path.trim().replace(/\/+$/, '')
  return trimmed === '' ? '/' : trimmed
}

/** Everything a profile generates, in one deterministic bundle. */
export type FixtureBundle = {
  readonly projects: readonly ProjectFixture[]
  readonly library: readonly LibraryItemFixture[]
  readonly assignments: readonly AssignmentFixture[]
  readonly exclusions: readonly ExclusionFixture[]
}
