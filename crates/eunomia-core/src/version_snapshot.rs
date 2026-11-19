//! Version snapshots of instructions and variants and their SHA-256
//! (spec §7.3, §9.8 ALG-070).
//!
//! When an instruction name, or a variant name or text, changes, the
//! app records a version (LIB-026). What a version stores is fixed by
//! §7.3's closing paragraph:
//!
//! > An instruction version snapshot stores the name and state
//! > (active or hidden). A variant version snapshot stores the name
//! > and text. The main-variant flag and the library order are not
//! > part of snapshots.
//!
//! ALG-070 then computes the SHA-256 of the JSON snapshot; if it
//! equals the last version's hash, no version is created — the same
//! ruling as spec §3.2 D-16, "a version is created only on a real
//! change". This module builds the snapshot string and its hash; the
//! comparison against the last version and the no-version decision
//! are the store's (the `revisions` table, §6.2), not made here.
//!
//! # Exclusion by construction
//!
//! The snapshot's content is exactly the functions' parameter list:
//! a name and a state, or a name and a text. There is no
//! main-flag parameter (and the domain
//! [`Variant`](crate::library::Variant) type carries no
//! `is_main` field at all — DECISIONS.md D-042: the flag lives in the
//! store's `instruction_variants.is_main` column), no position
//! parameter, no id parameter, no variant-list parameter. Two rows
//! that differ only in the main flag, the library order, or any
//! identity column therefore cannot produce different snapshots —
//! the tests pin this from the domain side. What the parameters
//! cannot carry, the hash cannot hash.
//!
//! # The canonical JSON
//!
//! ALG-070 says "a JSON snapshot" but no JSON literal; the exact
//! spelling is pinned here (ruling recorded in DECISIONS.md):
//!
//! ```text
//! {"name":"Be concise","state":"active"}
//! {"name":"Main","text":"Be brief."}
//! ```
//!
//! - compact form, no whitespace, fixed field order (name first —
//!   the §7.3 sentence's own order; state/text second);
//! - the keys are the spec's own snapshot words: `name` for both the
//!   instruction's title (`Instruction::title`) and the variant's
//!   name (`Variant::name`), `state` for the visibility state,
//!   `text` for the variant text (`Variant::content` — the §2 and
//!   DB-006 word, not the Rust field name);
//! - the state's JSON spelling is the schema §6.2 CHECK literal,
//!   [`InstructionState::as_str`] — the same canon the domain serde
//!   forms use (D-042);
//! - string values are escaped in exactly the form
//!   `serde_json::to_string` produces for the same data (`\"`, `\\`,
//!   the five short escapes `\b \t \n \f \r`, other control
//!   characters as lowercase `\u00xx`, every other code point
//!   verbatim in UTF-8). The dev-dependency pins this byte for byte
//!   in the tests: a store-side re-serialization of a snapshot with
//!   `serde_json` can never produce a different hash. Byte-equality
//!   with serde_json is why the emitter needs no framing of its own —
//!   quoting and commas make the mapping data → JSON injective, so
//!   unlike the build hash (D-064) no length framing is layered on.
//!
//! # Why the emitter is hand-rolled (the dependency ruling)
//!
//! Building JSON at runtime normally means `serde_json`, but the
//! spec pins core's dependencies (§5.2: "only `serde`, `sha2`,
//! `thiserror`"; DECISIONS.md D-017; D-042 kept `serde_json`
//! dev-only for exactly this kind of test-side pinning). Adopting it
//! at runtime would breach the allow-list, and for two fixed
//! two-field shapes a general JSON library is not needed: the
//! emitter is ~25 lines over a closed set of forms, in the spirit of
//! [`crate::hash`]'s hand-rolled hex renderer (D-064). It is NOT a
//! general JSON writer — do not extend it ad hoc; a third snapshot
//! shape or a wire format is a DECISIONS.md event.
//!
//! # The hash
//!
//! Plain SHA-256 over the snapshot's UTF-8 bytes (ALG-070 names no
//! framing and none is needed — see above), rendered as the app's
//! canonical 64-character lowercase hex (the spelling of
//! `revisions.snapshot_hash` comparisons and of
//! [`crate::hash::build_hash`]'s output; D-080's hash8 truncates
//! exactly this form). The snapshot and hash functions are offered
//! pairwise because the store persists both columns (§6.2
//! `revisions.snapshot`, `revisions.snapshot_hash`); the hash
//! functions hash exactly what their JSON twins build.
//!
//! # Boundary
//!
//! All four functions are total: any `&str` — empty, control
//! characters, NUL, any valid UTF-8 — produces a snapshot string and
//! a hash, never a panic. Content rules (§6.9 lengths, the reserved
//! marker lines, DB-006) are the store's write boundary, not a
//! snapshot concern: a snapshot is a faithful record of what is
//! stored, valid or not. No `AppError` mapping lives here (D-062's
//! boundary posture); nothing here reads a clock, a file, or a
//! database (D-006/D-017).
//!
//! # Examples
//!
//! ```
//! use eunomia_core::library::InstructionState;
//! use eunomia_core::version_snapshot::{
//!     instruction_snapshot, instruction_snapshot_hash, variant_snapshot,
//!     variant_snapshot_hash,
//! };
//!
//! // The instruction snapshot: name and state, nothing else.
//! assert_eq!(
//!     instruction_snapshot("Be concise", InstructionState::Active),
//!     "{\"name\":\"Be concise\",\"state\":\"active\"}",
//! );
//!
//! // The variant snapshot: name and text.
//! assert_eq!(
//!     variant_snapshot("Main", "Be brief."),
//!     "{\"name\":\"Main\",\"text\":\"Be brief.\"}",
//! );
//!
//! // Hashes are 64-character lowercase hex, deterministic per data…
//! let hash = variant_snapshot_hash("Main", "Be brief.");
//! assert_eq!(hash.len(), 64);
//! assert_eq!(hash, variant_snapshot_hash("Main", "Be brief."));
//!
//! // …and hiding the instruction (a state change) changes the hash:
//! assert_ne!(
//!     instruction_snapshot_hash("Be concise", InstructionState::Active),
//!     instruction_snapshot_hash("Be concise", InstructionState::Hidden),
//! );
//! ```

use sha2::{Digest, Sha256};

use crate::hash::to_lowercase_hex;
use crate::library::InstructionState;

/// Builds an instruction's version snapshot (spec §7.3, §9.8
/// ALG-070): the canonical JSON of the instruction's name and state —
/// nothing else (see the module docs for the exact form).
///
/// The name is `Instruction::title`; the JSON key is the spec's word
/// `name`. The state arrives as the domain enum, so its JSON spelling
/// is the §6.2 CHECK literal by construction.
///
/// # Examples
///
/// ```
/// use eunomia_core::library::InstructionState;
/// use eunomia_core::version_snapshot::instruction_snapshot;
///
/// assert_eq!(
///     instruction_snapshot("Be concise", InstructionState::Hidden),
///     "{\"name\":\"Be concise\",\"state\":\"hidden\"}",
/// );
/// ```
#[must_use]
pub fn instruction_snapshot(title: &str, state: InstructionState) -> String {
    let mut out = String::with_capacity(title.len() + 32);
    out.push_str("{\"name\":");
    push_json_string(&mut out, title);
    out.push_str(",\"state\":");
    push_json_string(&mut out, state.as_str());
    out.push('}');
    out
}

/// Computes an instruction's version-snapshot hash (spec §9.8
/// ALG-070): the SHA-256 of [`instruction_snapshot`]'s output, as
/// 64-character lowercase hex.
///
/// # Examples
///
/// ```
/// use eunomia_core::library::InstructionState;
/// use eunomia_core::version_snapshot::instruction_snapshot_hash;
///
/// let hash = instruction_snapshot_hash("Be concise", InstructionState::Active);
/// assert_eq!(hash.len(), 64);
/// assert!(
///     hash.bytes()
///         .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
/// );
/// ```
#[must_use]
pub fn instruction_snapshot_hash(title: &str, state: InstructionState) -> String {
    snapshot_hash(&instruction_snapshot(title, state))
}

/// Builds a variant's version snapshot (spec §7.3, §9.8 ALG-070):
/// the canonical JSON of the variant's name and text — nothing else
/// (see the module docs for the exact form).
///
/// The text is `Variant::content` as stored, raw and unnormalized
/// (the ALG-002 normalization is a build-pipeline step, D-042); the
/// JSON key is the spec's word `text`. The main-variant flag has no
/// parameter here and no field on
/// [`Variant`](crate::library::Variant): it cannot reach the
/// snapshot.
///
/// # Examples
///
/// ```
/// use eunomia_core::version_snapshot::variant_snapshot;
///
/// assert_eq!(
///     variant_snapshot("Main", "Be brief."),
///     "{\"name\":\"Main\",\"text\":\"Be brief.\"}",
/// );
/// // An empty text is a legitimate snapshot:
/// assert_eq!(variant_snapshot("Main", ""), "{\"name\":\"Main\",\"text\":\"\"}");
/// ```
#[must_use]
pub fn variant_snapshot(name: &str, content: &str) -> String {
    let mut out = String::with_capacity(name.len() + content.len() + 32);
    out.push_str("{\"name\":");
    push_json_string(&mut out, name);
    out.push_str(",\"text\":");
    push_json_string(&mut out, content);
    out.push('}');
    out
}

/// Computes a variant's version-snapshot hash (spec §9.8 ALG-070):
/// the SHA-256 of [`variant_snapshot`]'s output, as 64-character
/// lowercase hex.
///
/// # Examples
///
/// ```
/// use eunomia_core::version_snapshot::variant_snapshot_hash;
///
/// let hash = variant_snapshot_hash("Main", "");
/// assert_eq!(hash.len(), 64);
/// // A text change is a hash change — the trigger ALG-070 names:
/// assert_ne!(hash, variant_snapshot_hash("Main", " "));
/// ```
#[must_use]
pub fn variant_snapshot_hash(name: &str, content: &str) -> String {
    snapshot_hash(&variant_snapshot(name, content))
}

/// The SHA-256 of a snapshot's UTF-8 bytes, in the app's canonical
/// lowercase hex ([`crate::hash`]'s renderer — the same spelling
/// every stored SHA-256 uses).
fn snapshot_hash(json: &str) -> String {
    to_lowercase_hex(&Sha256::digest(json.as_bytes()))
}

/// Appends `value` as a quoted JSON string, escaped in exactly the
/// form `serde_json::to_string` produces (the module docs pin the
/// table): the two structural characters and the five named control
/// characters get short escapes, every other control character a
/// lowercase `\u00xx`, everything else passes through verbatim in
/// UTF-8. `&str` is valid UTF-8, so lone surrogates — the one input
/// class JSON escaping cannot carry raw — are unrepresentable.
fn push_json_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{0009}' => out.push_str("\\t"),
            '\u{000A}' => out.push_str("\\n"),
            '\u{000C}' => out.push_str("\\f"),
            '\u{000D}' => out.push_str("\\r"),
            ch if (ch as u32) < 0x20 => {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                let byte = ch as u32 as u8;
                out.push_str("\\u00");
                out.push(HEX[usize::from(byte >> 4)] as char);
                out.push(HEX[usize::from(byte & 0x0F)] as char);
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::library::{Instruction, ItemId, Variant, VariantId};

    /// The test fixtures' canonical literals, hashed independently
    /// with Python's `hashlib` over the exact UTF-8 bytes (an oracle
    /// outside the sha2 crate), like `hash.rs`'s oracle test.
    #[test]
    fn oracle_hashes_match_an_independently_computed_sha256() {
        // {"name":"Be concise","state":"active"}
        assert_eq!(
            instruction_snapshot_hash("Be concise", InstructionState::Active),
            "129ec91f397116358e6ed4b9c42844c3132d19f5aaefefa728f565244928b92a"
        );
        // {"name":"Be concise","state":"hidden"}
        assert_eq!(
            instruction_snapshot_hash("Be concise", InstructionState::Hidden),
            "b244cb4be1c391e7db9290218536926c18f64850702b721d2180f583cdb4e3b8"
        );
        // {"name":"Main","text":"Be brief."}
        assert_eq!(
            variant_snapshot_hash("Main", "Be brief."),
            "4f4c2ae7691504ab5e1d0876d104b7eb830f984f17b889acb688ce22bf7fa2df"
        );
        // {"name":"Main","text":""}
        assert_eq!(
            variant_snapshot_hash("Main", ""),
            "7f5342cc2555751322c7bd45988894d0006cb0c5614644649770805e29a32120"
        );
    }

    #[test]
    fn the_instruction_snapshot_is_the_exact_canonical_json() {
        // Field order (name, state), compact form, the §6.2 state
        // literals:
        assert_eq!(
            instruction_snapshot("Be concise", InstructionState::Active),
            "{\"name\":\"Be concise\",\"state\":\"active\"}"
        );
        assert_eq!(
            instruction_snapshot("Be concise", InstructionState::Hidden),
            "{\"name\":\"Be concise\",\"state\":\"hidden\"}"
        );
    }

    #[test]
    fn the_variant_snapshot_is_the_exact_canonical_json() {
        // Field order (name, text), compact form:
        assert_eq!(
            variant_snapshot("Main", "Be brief."),
            "{\"name\":\"Main\",\"text\":\"Be brief.\"}"
        );
        // Empty text — LIB-014's empty variant is a legitimate
        // snapshot, with its own hash:
        assert_eq!(
            variant_snapshot("Main", ""),
            "{\"name\":\"Main\",\"text\":\"\"}"
        );
    }

    #[test]
    fn identical_data_builds_identical_json_and_hash() {
        // Acceptance: the same data always produces the same snapshot
        // and the same hash — what D-16's "no version without a real
        // change" comparison relies on.
        for state in [InstructionState::Active, InstructionState::Hidden] {
            let first = instruction_snapshot("Be concise", state);
            let second = instruction_snapshot("Be concise", state);
            assert_eq!(first, second);
            assert_eq!(
                instruction_snapshot_hash("Be concise", state),
                instruction_snapshot_hash("Be concise", state)
            );
        }
        let first = variant_snapshot("Main", "Be brief.");
        assert_eq!(first, variant_snapshot("Main", "Be brief."));
        assert_eq!(
            variant_snapshot_hash("Main", "Be brief."),
            variant_snapshot_hash("Main", "Be brief.")
        );
    }

    #[test]
    fn the_main_flag_and_identity_cannot_change_a_variant_hash() {
        // Acceptance: the main-variant flag does not change the hash.
        // In the store the flag is `instruction_variants.is_main`
        // (§6.2); on the domain side it has no field at all (D-042),
        // and the snapshot functions take no flag parameter — so two
        // rows differing only in `is_main` ARE the same `(name,
        // content)` pair here, and their hashes are equal by
        // construction. The same holds for the variant id and the
        // instruction's variant list: not snapshot parameters.
        let left = Variant {
            id: VariantId::new("v1".to_owned()),
            name: "Main".to_owned(),
            content: "Be brief.".to_owned(),
        };
        let right = Variant {
            // The only difference a store row may carry: the identity
            // (and, unrepresentably, `is_main` — no field exists).
            id: VariantId::new("v2".to_owned()),
            ..left.clone()
        };
        assert_ne!(left.id, right.id);
        // One shared builder call each, pinned against the SAME
        // concrete literals the exact-JSON and oracle tests use — so
        // the assertion can fail: if a flag, an id, or any other
        // non-snapshot fact ever reached the snapshot, the literal
        // and the hash stop matching.
        assert_eq!(
            variant_snapshot(&right.name, &right.content),
            "{\"name\":\"Main\",\"text\":\"Be brief.\"}"
        );
        assert_eq!(
            variant_snapshot_hash(&right.name, &right.content),
            "4f4c2ae7691504ab5e1d0876d104b7eb830f984f17b889acb688ce22bf7fa2df"
        );
    }

    #[test]
    fn a_variants_list_change_cannot_change_an_instruction_hash() {
        // The instruction snapshot is (name, state); the variants
        // ride along on the domain value but are not snapshot
        // parameters — adding, renaming or reordering them (the
        // library order is explicitly outside snapshots) cannot move
        // the instruction's hash.
        let with_variants = Instruction {
            id: ItemId::new("i1".to_owned()),
            title: "Be concise".to_owned(),
            state: InstructionState::Active,
            variants: vec![
                Variant {
                    id: VariantId::new("v1".to_owned()),
                    name: "Main".to_owned(),
                    content: "Be brief.".to_owned(),
                },
                Variant {
                    id: VariantId::new("v2".to_owned()),
                    name: "Strict".to_owned(),
                    content: "Be very brief.".to_owned(),
                },
            ],
        };
        let without_variants = Instruction {
            variants: vec![],
            ..with_variants.clone()
        };
        assert_ne!(with_variants.variants, without_variants.variants);
        // Pinned against the same concrete literals the exact-JSON
        // and oracle tests use — a leaked variant list (or any other
        // fact outside the parameter list) changes the string and
        // the hash, and the pins fail:
        assert_eq!(
            instruction_snapshot(&with_variants.title, with_variants.state),
            "{\"name\":\"Be concise\",\"state\":\"active\"}"
        );
        assert_eq!(
            instruction_snapshot_hash(&with_variants.title, with_variants.state),
            "129ec91f397116358e6ed4b9c42844c3132d19f5aaefefa728f565244928b92a"
        );
    }

    #[test]
    fn a_name_change_changes_the_hash() {
        // Instruction name:
        assert_ne!(
            instruction_snapshot_hash("Be concise", InstructionState::Active),
            instruction_snapshot_hash("Be very concise", InstructionState::Active)
        );
        // Variant name (same text):
        assert_ne!(
            variant_snapshot_hash("Main", "Be brief."),
            variant_snapshot_hash("Strict", "Be brief.")
        );
        // A name change is also a snapshot change:
        assert_ne!(
            variant_snapshot("Main", "Be brief."),
            variant_snapshot("Strict", "Be brief.")
        );
    }

    #[test]
    fn a_state_change_changes_the_hash() {
        // Hiding is a real change (LIB-005 → a `hide` version, ALG-071):
        assert_ne!(
            instruction_snapshot_hash("Be concise", InstructionState::Active),
            instruction_snapshot_hash("Be concise", InstructionState::Hidden)
        );
        assert_ne!(
            instruction_snapshot("Be concise", InstructionState::Active),
            instruction_snapshot("Be concise", InstructionState::Hidden)
        );
    }

    #[test]
    fn a_text_change_changes_the_hash() {
        assert_ne!(
            variant_snapshot_hash("Main", "Be brief."),
            variant_snapshot_hash("Main", "Be very brief.")
        );
        // Snapshots carry the stored form byte for byte: whitespace
        // is not normalized away (ALG-002 normalization is a
        // build-pipeline step, D-042)…
        assert_ne!(
            variant_snapshot_hash("Main", "Be brief."),
            variant_snapshot_hash("Main", "Be brief. ")
        );
        // …and a trailing line break alone is a real change:
        assert_ne!(
            variant_snapshot_hash("Main", "Be brief."),
            variant_snapshot_hash("Main", "Be brief.\n")
        );
    }

    #[test]
    fn an_empty_text_hashes_differently_from_a_whitespace_text() {
        assert_ne!(
            variant_snapshot_hash("Main", ""),
            variant_snapshot_hash("Main", " ")
        );
    }

    #[test]
    fn string_values_escape_like_serde_json() {
        // A hand-written literal pin: quote, backslash, line break,
        // tab, verbatim emoji and Cyrillic (no \u escapes for
        // non-ASCII, DEL passes through).
        let text = "a\"b\\c\nd\te🚀Привет";
        assert_eq!(
            variant_snapshot("esc", text),
            "{\"name\":\"esc\",\"text\":\"a\\\"b\\\\c\\nd\\te🚀Привет\"}"
        );
        assert_eq!(
            variant_snapshot_hash("esc", text),
            // Independently computed (Python hashlib) over exactly
            // this JSON's UTF-8 bytes:
            "ffe79869a8606003b1669230ea55b7c3f95caaf130de75eda6578bf02de87851"
        );
        // The same on the instruction's name:
        assert_eq!(
            instruction_snapshot("Say \"hi\"", InstructionState::Hidden),
            "{\"name\":\"Say \\\"hi\\\"\",\"state\":\"hidden\"}"
        );
    }

    /// The serde side of the byte-compat oracle: the emitter's output
    /// must equal `serde_json::to_string` of the same (name, text)
    /// pair with the same field order, and must parse back to it.
    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct CrossCheck {
        name: String,
        text: String,
    }

    #[test]
    fn every_control_character_escapes_exactly_like_serde_json() {
        // Exhaustive over the escape table: all 32 ASCII control
        // characters (the five short forms, 27 \u00xx forms), plus
        // the structural characters, DEL (escaped by no one), and
        // verbatim non-ASCII samples. The dev-dependency is the
        // oracle: the snapshot strings stay byte-identical to what a
        // store-side serde_json serialization produces.
        let control: String = (0u8..32).map(char::from).collect();
        let samples = [
            control.as_str(),
            "\"quote\"",
            "back\\slash",
            "mixed\"quote\\and\nnewline\ttab",
            "\u{007f}del\u{007f}",
            "🚀Привет Vectør — em\u{2013}dash",
            "",
        ];
        for text in samples {
            for name in ["Main", "na\"me\\", "🚀"] {
                let snapshot = variant_snapshot(name, text);
                let expected = serde_json::to_string(&CrossCheck {
                    name: name.to_owned(),
                    text: text.to_owned(),
                })
                .unwrap();
                assert_eq!(snapshot, expected, "name={name:?} text={text:?}");
                // And the emitted JSON is valid — it parses back to
                // the very pair it was built from:
                let parsed: CrossCheck = serde_json::from_str(&snapshot).unwrap();
                assert_eq!(
                    parsed,
                    CrossCheck {
                        name: name.to_owned(),
                        text: text.to_owned(),
                    }
                );
            }
        }
        // The same byte-compat on the instruction form (state is the
        // schema literal, so only the name needs the check):
        for name in ["Be \"concise\"", "tab\tname", "🚀"] {
            for state in [InstructionState::Active, InstructionState::Hidden] {
                let snapshot = instruction_snapshot(name, state);
                let expected = format!(
                    "{{\"name\":{},\"state\":\"{}\"}}",
                    serde_json::to_string(&name).unwrap(),
                    state.as_str(),
                );
                assert_eq!(snapshot, expected, "name={name:?}");
            }
        }
    }

    #[test]
    fn the_maximum_variant_text_builds_deterministically() {
        // DB-006's text limit is the store's gate, but a snapshot of
        // a boundary-sized text must still be total and stable:
        let text = "a".repeat(262_144);
        let hash = variant_snapshot_hash("Main", &text);
        assert_eq!(hash, variant_snapshot_hash("Main", &text));
        assert_eq!(hash.len(), 64);
        // One character more is a different snapshot:
        assert_ne!(hash, variant_snapshot_hash("Main", &format!("{text}b")));
    }
}
