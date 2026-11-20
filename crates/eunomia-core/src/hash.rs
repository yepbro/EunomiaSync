//! The project build hash (spec §6.6 DB-020, §9.3 ALG-033).
//!
//! DB-020 fixes the hash of a project's build:
//!
//! ```text
//! sha256('v1' + scheme + target path + extra-file flags and positions
//!        + build text without the creation marker)
//! ```
//!
//! The value is the project's `last_synced_hash` fingerprint: the
//! dirty flag (DB-021) compares it with the hash of the last
//! successful sync, and a sync stores it after a `written` or
//! `unchanged` status (DB-024). ALG-033 fixes what is hashed — the
//! `managed` content *without* the creation marker: the marker carries
//! the run's wall-clock time (ALG-032), which must not make every
//! rebuild dirty.
//!
//! # The marker is excluded by construction
//!
//! [`build_hash`] takes the managed text WITHOUT the marker and has no
//! time, zone or marker parameter at all — the run time physically
//! cannot reach the hash. Today the markerless managed text is
//! [`build_body`](crate::body::build_body)'s output (the §9.2 body
//! never contains the marker, DECISIONS.md D-053); when the ALG-030
//! file assembly joins the extra-file reading lines around the body,
//! its `managed` string (still without the marker) is what gets passed
//! here. The unit tests prove the property end to end: full contents
//! that differ only in their marker line hash identically.
//!
//! # The byte stream (what exactly is hashed)
//!
//! The spec's `+` chain names the fields and their order but no
//! separators, and a plain concatenation of these fields is ambiguous:
//! a target path may end in `…falseend` while a managed text starts
//! with `trueendtrueend`, so two DIFFERENT inputs can produce the very
//! same byte string (a unit test pins such a pair). The mapping
//! input → byte stream is therefore made injective by framing every
//! field with its byte length first (an 8-byte little-endian `u64`,
//! then the bytes), in the DB-020 field order, behind the literal
//! `v1` tag — a constant prefix that needs no framing and versions the
//! formula: a future `v2` formula starts a different tag and cannot
//! collide with `v1` hashes.
//!
//! Field encodings, all canonical:
//!
//! - **scheme**: the schema literal `"A"`/`"B"`/`"C"`
//!   ([`Scheme::as_str`](crate::project::Scheme::as_str));
//! - **target path**: the path's own platform bytes
//!   (`as_os_str().as_encoded_bytes()`), not normalized — the
//!   snapshot's project root is canonical (DB-030) and the target path
//!   derives from it, so the same project yields the same bytes on
//!   every run of the same platform; comparing spellings across
//!   platforms is not this hash's job. The base name reaches the hash
//!   through the target path only (`<root>/<base_name>` or
//!   `<root>/<STEM>.project.md`), exactly as DB-020's field list says;
//! - **extra-file flags and positions**: both extra files, in the §6.2
//!   column order (`extra_local` first, then `extra_project`), each as
//!   its enabled flag (`"true"`/`"false"`, the §6.5 JSON spelling)
//!   followed by its position (`"start"`/`"end"`). They are included
//!   under every scheme: the formula has no scheme qualifier, and
//!   DB-023's "changing the scheme or the file name makes the project
//!   dirty" is the same reading — the hash fingerprints the project's
//!   build configuration, not just the bytes of one file;
//! - **build text**: the markerless managed string, byte for byte as
//!   built — no normalization happens here (the build already applied
//!   ALG-002 to every variant text; ALG-030 joins the parts).
//!
//! # Output
//!
//! Lowercase hexadecimal, 64 characters — the spelling of
//! `projects.last_synced_hash` (DB-020) and of the SHA-256 values the
//! backup checks compare (BKP-010).
//!
//! The build hash is not the only SHA-256 this crate owns:
//! [`Sha256Hex`] is the PLAIN byte-stream digest behind BKP-010's
//! backup checks — the same digest arithmetic and the same spelling,
//! without the `v1` framing. `eunomia-fs` streams file bytes through
//! it, so file-system code hashes without ever naming the `sha2`
//! crate (the §5.2 `fs` row lists `core`, not `sha2`; D-017's
//! per-crate allow-lists stay intact).

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::project::{ExtraFile, Scheme};

/// The formula-version tag of DB-020's hash: `'v1'`.
const VERSION_TAG: &[u8] = b"v1";

/// Computes a project's build hash (spec §6.6 DB-020, §9.3 ALG-033).
///
/// The five inputs are exactly DB-020's fields, in the formula's
/// order: the write scheme, the target file's path, both extra-file
/// options, and the managed text **without the creation marker** (see
/// the module docs for the byte stream and each field's encoding).
/// There is deliberately no time, zone or marker parameter: ALG-032's
/// run time must not reach the hash, and the marker never being an
/// input is what makes that a structural guarantee.
///
/// The call is pure — no clock, no I/O — so the same inputs always
/// produce the same 64-character lowercase hex string, which is what
/// the DB-021 dirty comparison and the DB-024 `last_synced_hash`
/// update rely on.
///
/// # Examples
///
/// ```
/// use std::path::Path;
///
/// use eunomia_core::hash::build_hash;
/// use eunomia_core::project::{ExtraFile, ExtraFilePosition, Scheme};
///
/// let (local, project) = (
///     ExtraFile { enabled: true, position: ExtraFilePosition::Start },
///     ExtraFile { enabled: false, position: ExtraFilePosition::End },
/// );
///
/// let hash =
///     build_hash(Scheme::A, Path::new("/repos/demo/AGENTS.md"), local, project, "Be brief.");
/// // Sixty-four lowercase hex characters:
/// assert_eq!(hash.len(), 64);
/// assert!(hash.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
///
/// // The same input hashes to the same value…
/// assert_eq!(
///     hash,
///     build_hash(Scheme::A, Path::new("/repos/demo/AGENTS.md"), local, project, "Be brief.")
/// );
/// // …and a managed-text change alone changes it:
/// assert_ne!(
///     hash,
///     build_hash(Scheme::A, Path::new("/repos/demo/AGENTS.md"), local, project, "Be very brief.")
/// );
/// ```
#[must_use]
pub fn build_hash(
    scheme: Scheme,
    target_path: &Path,
    extra_local: ExtraFile,
    extra_project: ExtraFile,
    managed_without_marker: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(VERSION_TAG);
    feed_framed(&mut hasher, scheme.as_str().as_bytes());
    feed_framed(&mut hasher, target_path.as_os_str().as_encoded_bytes());
    feed_framed(&mut hasher, flag_bytes(extra_local.enabled));
    feed_framed(&mut hasher, extra_local.position.as_str().as_bytes());
    feed_framed(&mut hasher, flag_bytes(extra_project.enabled));
    feed_framed(&mut hasher, extra_project.position.as_str().as_bytes());
    feed_framed(&mut hasher, managed_without_marker.as_bytes());
    to_lowercase_hex(&hasher.finalize())
}

/// Feeds one field: its byte length, then its bytes — the framing that
/// makes the byte stream injective over the inputs (see the module
/// docs).
fn feed_framed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

/// The canonical spelling of an extra-file enabled flag.
fn flag_bytes(enabled: bool) -> &'static [u8] {
    if enabled { b"true" } else { b"false" }
}

/// An incremental SHA-256 over a raw byte stream, rendered as the
/// canonical 64-character lowercase hexadecimal digest.
///
/// This is the plain file-content hash of spec §7.6 BKP-010 — NOT the
/// DB-020 build hash: [`build_hash`] feeds a framed, versioned byte
/// stream (`v1` + length-framed fields), while a `Sha256Hex` digests
/// exactly the bytes it is handed and nothing else — the digest of a
/// file's content, byte for byte. The output spelling is the same
/// canonical 64 lowercase hex characters as every compared SHA-256 in
/// the app, so a digest recorded by the store layer and a digest
/// computed by the file-system layer compare by exact string
/// equality (the backup dedup contract, BKP-010: equal digests mean
/// equal content).
///
/// The API is incremental on purpose: `eunomia-fs` streams a file
/// through [`update`](Sha256Hex::update) in fixed-size chunks — no
/// whole file is ever loaded into memory — and reads the digest with
/// [`finalize_hex`](Sha256Hex::finalize_hex). The type is pure (no
/// I/O, no clock): it owns only the digest arithmetic, keeping
/// `sha2` a core-internal detail — file-system code hashes through
/// this wrapper and never depends on the hashing crate itself (the
/// §5.2 `fs` row lists `core`, not `sha2`; DECISIONS.md D-017's
/// per-crate allow-lists stay intact). The DB-020 build hash is
/// untouched by this wrapper: its formula, framing and `v1` tag live
/// only in [`build_hash`].
///
/// # Examples
///
/// ```
/// use eunomia_core::hash::Sha256Hex;
///
/// let mut hasher = Sha256Hex::new();
/// hasher.update(b"a");
/// hasher.update(b"bc");
/// // The FIPS 180-2 vector for "abc", in the canonical spelling:
/// assert_eq!(
///     hasher.finalize_hex(),
///     "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
/// );
/// ```
#[derive(Debug)]
pub struct Sha256Hex {
    /// The crate-internal digest state; the wrapper exists so callers
    /// never name the hashing crate or the digest's byte form.
    inner: Sha256,
}

impl Sha256Hex {
    /// Starts a fresh digest over zero bytes so far.
    #[must_use = "a dropped hasher digests nothing"]
    pub fn new() -> Self {
        Self {
            inner: Sha256::new(),
        }
    }

    /// Feeds the next chunk of bytes into the digest. Chunks may come
    /// in any sizes and any counts: the digest depends only on the
    /// total byte stream, never on where the chunk boundaries fell
    /// (pinned by tests against independently computed FIPS vectors).
    pub fn update(&mut self, bytes: &[u8]) {
        self.inner.update(bytes);
    }

    /// Finishes the digest and renders it as the canonical 64
    /// lowercase hexadecimal characters (see the type's docs).
    #[must_use = "the digest is the only output; dropping it hashes for nothing"]
    pub fn finalize_hex(self) -> String {
        to_lowercase_hex(&self.inner.finalize())
    }
}

impl Default for Sha256Hex {
    fn default() -> Self {
        Self::new()
    }
}

/// Renders digest bytes as lowercase hexadecimal. Hand-rolled: a `hex`
/// crate would leave the dependency allow-list pinned by DECISIONS.md
/// D-017 (serde, thiserror, sha2), and sixteen lines do not justify a
/// DECISIONS-level addition. Shared with the version-snapshot module,
/// whose ALG-070 hashes use the same spelling.
pub(crate) fn to_lowercase_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0x0F)] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::marker::MarkerTime;
    use crate::marker::creation_marker_line;
    use crate::project::ExtraFilePosition;

    const PATH: &str = "/repos/demo/AGENTS.md";
    const MANAGED: &str = "## Style\n\nBe brief.";

    fn extras() -> (ExtraFile, ExtraFile) {
        (
            ExtraFile {
                enabled: true,
                position: ExtraFilePosition::Start,
            },
            ExtraFile {
                enabled: false,
                position: ExtraFilePosition::End,
            },
        )
    }

    fn hash_with(
        scheme: Scheme,
        path: &str,
        extras: (ExtraFile, ExtraFile),
        managed: &str,
    ) -> String {
        build_hash(scheme, Path::new(path), extras.0, extras.1, managed)
    }

    #[test]
    fn oracle_vector_matches_an_independently_computed_sha256() {
        // Expected value computed with Python's hashlib (an independent
        // SHA-256 implementation) over this exact framed byte stream:
        // b"v1" + LE64(1) + b"A" + LE64(21) + b"/repos/demo/AGENTS.md"
        // + LE64(4) + b"true" + LE64(5) + b"start" + LE64(5) + b"false"
        // + LE64(3) + b"end" + LE64(19) + b"## Style\n\nBe brief."
        // — pinning the tag, the field order, the framing and every
        // field literal in one value.
        assert_eq!(
            hash_with(Scheme::A, PATH, extras(), MANAGED),
            "d2a2446b2437df4c64a4db2b3a49a45e519fddf98e8718ac0fb9f682f9b83d27"
        );
    }

    #[test]
    fn the_hash_is_64_lowercase_hex_characters() {
        let hash = hash_with(Scheme::C, "/r/CLAUDE.project.md", extras(), "");
        assert_eq!(hash.len(), 64);
        assert!(
            hash.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
    }

    #[test]
    fn identical_inputs_hash_identically() {
        assert_eq!(
            hash_with(Scheme::A, PATH, extras(), MANAGED),
            hash_with(Scheme::A, PATH, extras(), MANAGED)
        );
    }

    #[test]
    fn changing_the_scheme_alone_changes_the_hash() {
        for scheme in [Scheme::B, Scheme::C] {
            assert_ne!(
                hash_with(scheme, PATH, extras(), MANAGED),
                hash_with(Scheme::A, PATH, extras(), MANAGED)
            );
        }
    }

    #[test]
    fn changing_the_target_path_alone_changes_the_hash() {
        // A different repository root:
        assert_ne!(
            hash_with(Scheme::A, "/repos/other/AGENTS.md", extras(), MANAGED),
            hash_with(Scheme::A, PATH, extras(), MANAGED)
        );
        // A different base name (DB-023: changing the file name makes
        // the project dirty — the base name reaches the hash through
        // the target path):
        assert_ne!(
            hash_with(Scheme::A, "/repos/demo/CLAUDE.md", extras(), MANAGED),
            hash_with(Scheme::A, PATH, extras(), MANAGED)
        );
        // The scheme-C separate file:
        assert_ne!(
            hash_with(
                Scheme::A,
                "/repos/demo/AGENTS.project.md",
                extras(),
                MANAGED
            ),
            hash_with(Scheme::A, PATH, extras(), MANAGED)
        );
    }

    #[test]
    fn changing_each_extra_flag_or_position_alone_changes_the_hash() {
        let (local, project) = extras();
        // extra_local.enabled:
        assert_ne!(
            hash_with(
                Scheme::A,
                PATH,
                (
                    ExtraFile {
                        enabled: false,
                        ..local
                    },
                    project
                ),
                MANAGED
            ),
            hash_with(Scheme::A, PATH, extras(), MANAGED)
        );
        // extra_local.position:
        assert_ne!(
            hash_with(
                Scheme::A,
                PATH,
                (
                    ExtraFile {
                        position: ExtraFilePosition::End,
                        ..local
                    },
                    project
                ),
                MANAGED
            ),
            hash_with(Scheme::A, PATH, extras(), MANAGED)
        );
        // extra_project.enabled:
        assert_ne!(
            hash_with(
                Scheme::A,
                PATH,
                (
                    local,
                    ExtraFile {
                        enabled: true,
                        ..project
                    }
                ),
                MANAGED
            ),
            hash_with(Scheme::A, PATH, extras(), MANAGED)
        );
        // extra_project.position:
        assert_ne!(
            hash_with(
                Scheme::A,
                PATH,
                (
                    local,
                    ExtraFile {
                        position: ExtraFilePosition::Start,
                        ..project
                    }
                ),
                MANAGED
            ),
            hash_with(Scheme::A, PATH, extras(), MANAGED)
        );
        // The extras count under every scheme — the formula has no
        // scheme qualifier, even where the built file ignores them
        // (under B and C, ALG-030 adds no reading lines):
        assert_ne!(
            hash_with(
                Scheme::B,
                PATH,
                (
                    local,
                    ExtraFile {
                        enabled: true,
                        ..project
                    }
                ),
                MANAGED
            ),
            hash_with(Scheme::B, PATH, (local, project), MANAGED)
        );
    }

    #[test]
    fn changing_the_managed_text_alone_changes_the_hash() {
        assert_ne!(
            hash_with(Scheme::A, PATH, extras(), "## Style\n\nBe very brief."),
            hash_with(Scheme::A, PATH, extras(), MANAGED)
        );
        // An empty build is a legitimate, different input:
        assert_ne!(
            hash_with(Scheme::A, PATH, extras(), ""),
            hash_with(Scheme::A, PATH, extras(), MANAGED)
        );
        // Even a single trailing line break differs (the managed text
        // is hashed byte for byte, no normalization here):
        assert_ne!(
            hash_with(Scheme::A, PATH, extras(), "## Style\n\nBe brief.\n"),
            hash_with(Scheme::A, PATH, extras(), MANAGED)
        );
    }

    #[test]
    fn marker_only_differences_leave_the_hash_unchanged() {
        // The hash input is the managed text without the marker and
        // the function has no time parameter at all (ALG-032's run
        // time cannot reach it). The proof: full contents in the
        // ALG-030 shape — managed, an empty line, the marker, one line
        // break — whose ONLY difference is the marker line, formatted
        // by the real marker module.
        let markers = [
            creation_marker_line(
                MarkerTime {
                    year: 2026,
                    month: 11,
                    day: 17,
                    hour: 14,
                    minute: 3,
                    second: 22,
                },
                Some("Europe/Prague"),
                60,
            ),
            creation_marker_line(
                MarkerTime {
                    year: 2031,
                    month: 1,
                    day: 2,
                    hour: 3,
                    minute: 4,
                    second: 5,
                },
                Some("Europe/Prague"),
                60,
            ),
            // The D-23 UTC fallback form and a western zone are marker
            // differences just the same:
            creation_marker_line(
                MarkerTime {
                    year: 2026,
                    month: 11,
                    day: 17,
                    hour: 14,
                    minute: 3,
                    second: 22,
                },
                None,
                60,
            ),
            creation_marker_line(
                MarkerTime {
                    year: 2026,
                    month: 11,
                    day: 17,
                    hour: 8,
                    minute: 3,
                    second: 22,
                },
                Some("America/New_York"),
                -300,
            ),
        ];
        let contents: Vec<String> = markers
            .iter()
            .map(|marker| format!("{MANAGED}\n\n{marker}\n"))
            .collect();
        // They are different files on disk…
        for pair in contents.windows(2) {
            assert_ne!(pair[0], pair[1]);
        }
        // …but once the marker line is dropped they are the same
        // markerless remainder (ALG-033's drop rule, mirrored here)…
        let stripped: Vec<String> = contents.iter().map(|c| strip_last_marker_line(c)).collect();
        for pair in stripped.windows(2) {
            assert_eq!(pair[0], pair[1]);
        }
        // What the mirror yields, pinned byte for byte: the managed
        // text plus the empty line's break — dropping the marker line
        // removes that line alone, not the separator before it. The
        // remainder is what the drop rule leaves; the production hash
        // input is the managed string itself, and both are time-free
        // for the same structural reason: the marker never enters.
        assert_eq!(stripped[0], "## Style\n\nBe brief.\n");

        // …and therefore every hash is equal.
        let hashes: Vec<String> = stripped
            .iter()
            .map(|managed| hash_with(Scheme::A, PATH, extras(), managed))
            .collect();
        for pair in hashes.windows(2) {
            assert_eq!(pair[0], pair[1]);
        }
    }

    // ---- the plain byte-stream digest (Sha256Hex) ----------------------

    #[test]
    fn the_plain_digest_matches_the_fips_vectors() {
        // Independently computed SHA-256 values (FIPS 180-2 vectors,
        // re-checked with Python's hashlib): the wrapper digests raw
        // bytes with no framing of its own.
        let cases: [(&[u8], &str); 3] = [
            (
                b"",
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                b"abc",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
                "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
            ),
        ];
        for (input, expected) in cases {
            let mut hasher = Sha256Hex::new();
            hasher.update(input);
            assert_eq!(hasher.finalize_hex(), expected, "input: {input:?}");
        }
    }

    #[test]
    fn the_plain_digest_is_independent_of_the_chunking() {
        // The BKP-010 hash streams files in fixed chunks; the digest
        // must depend on the byte stream alone, never on where the
        // chunk boundaries fell (1-byte, 3-byte and whole-stream
        // feedings of the same content all agree, and agree with the
        // FIPS vector).
        let input = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        let expected = "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1";

        let whole = {
            let mut hasher = Sha256Hex::new();
            hasher.update(input);
            hasher.finalize_hex()
        };
        assert_eq!(whole, expected);

        let one_byte_chunks = {
            let mut hasher = Sha256Hex::new();
            for byte in input {
                hasher.update(std::slice::from_ref(byte));
            }
            hasher.finalize_hex()
        };
        assert_eq!(one_byte_chunks, expected);

        let odd_chunks = {
            let mut hasher = Sha256Hex::new();
            for chunk in input.chunks(7) {
                hasher.update(chunk);
            }
            hasher.finalize_hex()
        };
        assert_eq!(odd_chunks, expected);
    }

    #[test]
    fn the_plain_digest_spelling_is_64_lowercase_hex_characters() {
        let mut hasher = Sha256Hex::new();
        hasher.update(b"abc");
        let digest = hasher.finalize_hex();
        assert_eq!(digest.len(), 64);
        assert!(
            digest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "the digest must be lowercase hex, got: {digest}"
        );
    }

    #[test]
    fn a_default_hasher_is_a_fresh_hasher() {
        let mut hasher = Sha256Hex::default();
        hasher.update(b"");
        assert_eq!(
            hasher.finalize_hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn field_framing_separates_inputs_whose_plain_concatenation_is_identical() {
        // Two valid inputs whose naive '+' concatenations (the literal
        // reading of DB-020) are the same byte string: the second
        // path absorbs the first input's extra-file tokens, and its
        // managed text absorbs the rest. The premise is asserted here
        // so the test carries its own proof.
        let left = (
            "/r",
            (
                ExtraFile {
                    enabled: false,
                    position: ExtraFilePosition::End,
                },
                ExtraFile {
                    enabled: false,
                    position: ExtraFilePosition::End,
                },
            ),
            "trueendtrueendX",
        );
        let right = (
            "/rfalseendfalseend",
            (
                ExtraFile {
                    enabled: true,
                    position: ExtraFilePosition::End,
                },
                ExtraFile {
                    enabled: true,
                    position: ExtraFilePosition::End,
                },
            ),
            "X",
        );

        let plain = |path: &str, (local, project): (ExtraFile, ExtraFile), text: &str| {
            format!(
                "v1A{path}{}{}{}{}{text}",
                if local.enabled { "true" } else { "false" },
                local.position.as_str(),
                if project.enabled { "true" } else { "false" },
                project.position.as_str(),
            )
        };
        assert_eq!(
            plain(left.0, left.1, left.2),
            plain(right.0, right.1, right.2)
        );

        // The length framing tells the two inputs apart:
        assert_ne!(
            hash_with(Scheme::A, left.0, left.1, left.2),
            hash_with(Scheme::A, right.0, right.1, right.2)
        );
    }

    /// A minimal test-local mirror of ALG-033's drop rule — the last
    /// non-empty line that starts with `<!-- Created by EunomiaSync `
    /// and ends with ` -->`. The production scanner is a later task's
    /// module; this copy exists only to prove the hash-side exclusion.
    fn strip_last_marker_line(content: &str) -> String {
        const PREFIX: &str = "<!-- Created by EunomiaSync ";
        const SUFFIX: &str = " -->";
        let mut lines: Vec<&str> = content.lines().collect();
        let index = lines
            .iter()
            .rposition(|line| line.starts_with(PREFIX) && line.ends_with(SUFFIX))
            .expect("the fixture content carries a marker line");
        lines.remove(index);
        lines.join("\n")
    }
}
