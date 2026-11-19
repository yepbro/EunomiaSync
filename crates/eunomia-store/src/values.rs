//! Canonical TEXT value formats stored in the database (spec DB-002).
//!
//! Every identifier and timestamp column uses one of two text conventions:
//!
//! * row identifiers are UUIDv7 in the canonical lowercase hyphenated form
//!   ([`new_id`]);
//! * timestamps are UTC ISO-8601 strings with millisecond precision, for
//!   example `2026-11-17T13:03:22.123Z` ([`now_utc`]).
//!
//! Both helpers return plain [`String`]: TEXT is the storage contract, and
//! until a consumer needs a typed wrapper none is invented. The canonical
//! lowercase TEXT form of a UUID sorts identically to its binary form (in
//! ASCII, `0`–`9` precede `a`–`f` exactly as their nibble values do), so an
//! `ORDER BY` on an id column preserves creation order.
//!
//! The `uuid` dependency was added per DECISIONS.md D-031; the time
//! dependency behind [`now_utc`] is `jiff`, which replaced chrono per
//! DECISIONS.md D-035 (chrono is RUSTSEC-2026-0543, unmaintained).

use jiff::Timestamp;
use uuid::Uuid;

/// Generates a new row identifier (spec DB-002).
///
/// The identifier is a UUIDv7 in its canonical lowercase hyphenated TEXT
/// form, for example `0193a2aa-6c9e-7c8e-9c6d-1a2b3c4d5e6f`. UUIDv7 places
/// a Unix millisecond timestamp in the most significant bits, so the TEXT
/// form sorts by creation time.
///
/// Ordering guarantee (DECISIONS.md D-031): identifiers returned by this
/// function from the same process are strictly increasing in generation
/// order. [`Uuid::now_v7`] draws from a process-wide counter-backed
/// context (the RFC 9562 §6.2 counter method), which orders calls inside
/// a single millisecond and absorbs backward clock jitter. Across processes
/// there is no ordering guarantee; nothing in this crate may rely on one.
///
/// The guarantee covers ordering only: after a backward clock step the
/// context keeps the highest timestamp it has observed, so a freshly
/// generated id can embed a time ahead of the wall clock. Never read an
/// id as a time source — use [`now_utc`] for that.
pub fn new_id() -> String {
    Uuid::now_v7().to_string()
}

/// Returns the current UTC time as an ISO-8601 string (spec DB-002).
///
/// The format is fixed: `YYYY-MM-DDTHH:MM:SS.sssZ` — UTC, always exactly
/// three fractional digits and the `Z` suffix, for example
/// `2026-11-17T13:03:22.123Z`. Sub-millisecond precision is truncated
/// (not rounded): the `%.3f` directive of [`Timestamp::strftime`] is
/// documented to truncate toward zero (DECISIONS.md D-035).
pub fn now_utc() -> String {
    Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/// Whether `value` has the exact DB-002 TEXT shape —
/// `YYYY-MM-DDTHH:MM:SS.sssZ`, 24 characters: digits at every
/// non-punctuation position, `-`/`-`/`T`/`:`/`:`/`.`/`Z` at the fixed
/// ones (D-031/D-035).
///
/// The check is **character-level, not calendar-level**: `…-13-…`
/// (month 13) is shape-valid. Calendar validity is a different
/// concern with different owners (jiff rejects it when a value is
/// parsed as an instant; a filter bound that is shape-valid but
/// calendar-invalid simply matches nothing in the range comparison,
/// which stays well-defined).
///
/// This is the predicate behind every **string comparison over
/// timestamp columns**: two values of exactly this shape compare
/// lexicographically exactly as their instants compare chronologically
/// (fixed width, fixed offsets, always UTC `Z`). A value that merely
/// *parses* as RFC 3339 does not have that property —
/// `…T13:00:00Z` (no fractional digits) and `…T13:00:00.000Z` name
/// the same instant but compare unequal as text (`Z` > `.`) — so both
/// the query's date **bounds** and the read path's stored **cells**
/// (`action_log`'s `ts`) accept only this exact shape and reject
/// everything else instead of silently mis-ordering.
#[must_use = "the check is free; skipping it forfeits the lexicographic-ordering guarantee"]
pub fn is_db002_shape(value: &str) -> bool {
    /// The fixed punctuation of `YYYY-MM-DDTHH:MM:SS.sssZ`, by byte
    /// position.
    const PUNCTUATION: [(usize, u8); 7] = [
        (4, b'-'),
        (7, b'-'),
        (10, b'T'),
        (13, b':'),
        (16, b':'),
        (19, b'.'),
        (23, b'Z'),
    ];
    let bytes = value.as_bytes();
    bytes.len() == 24
        && PUNCTUATION
            .iter()
            .all(|(index, expected)| bytes[*index] == *expected)
        && bytes.iter().enumerate().all(|(index, byte)| {
            PUNCTUATION
                .iter()
                .any(|(punctuation, _)| *punctuation == index)
                || byte.is_ascii_digit()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Byte positions of the fixed punctuation in `YYYY-MM-DDTHH:MM:SS.sssZ`.
    const TIMESTAMP_PUNCTUATION: [(usize, u8); 7] = [
        (4, b'-'),
        (7, b'-'),
        (10, b'T'),
        (13, b':'),
        (16, b':'),
        (19, b'.'),
        (23, b'Z'),
    ];

    fn assert_timestamp_shape(value: &str) {
        let bytes = value.as_bytes();
        assert_eq!(bytes.len(), 24, "timestamp must be 24 characters: {value}");
        for (index, expected) in TIMESTAMP_PUNCTUATION {
            assert_eq!(bytes[index], expected, "byte {index} of {value}");
        }
        for (index, byte) in bytes.iter().enumerate() {
            let is_punctuation = TIMESTAMP_PUNCTUATION.iter().any(|(i, _)| *i == index);
            if !is_punctuation {
                assert!(
                    byte.is_ascii_digit(),
                    "byte {index} of {value} must be a digit"
                );
            }
        }
    }

    #[test]
    fn now_utc_matches_db002_format() {
        // Every call must produce the fixed shape — including the `Z` suffix
        // and exactly three fractional digits, whatever the clock's
        // sub-millisecond digits happen to be.
        for _ in 0..100 {
            assert_timestamp_shape(&now_utc());
        }
    }

    #[test]
    fn is_db002_shape_accepts_the_canonical_form_and_rejects_near_misses() {
        // The canonical shape (an oracle literal, not `now_utc` output).
        assert!(is_db002_shape("2026-11-17T13:03:22.123Z"));
        // The predicate is character-level, not calendar-level: month
        // 13 is shape-valid (every non-punctuation position is a
        // digit). Calendar validation is not this predicate's job — a
        // calendar-invalid bound simply matches nothing in the range
        // comparison, which is well-defined for any shape-conforming
        // string — and pinning the boundary keeps a future "fix" from
        // silently re-scoping it.
        assert!(is_db002_shape("2026-13-17T13:03:22.123Z"));
        // Near misses: each one parses as RFC 3339 (or is plainly a
        // date), yet none has the exact fixed-width shape whose
        // lexicographic order equals chronological order.
        for rejected in [
            "2026-11-17T13:03:22Z",          // no fractional digits
            "2026-11-17T13:03:22.12Z",       // two fractional digits
            "2026-11-17T13:03:22.1234Z",     // four fractional digits
            "2026-11-17T13:03:22.123+00:00", // explicit offset, no Z
            "2026-11-17t13:03:22.123Z",      // lowercase T
            "2026-11-17T13:03:22.123z",      // lowercase Z
            "2026-11-17 13:03:22.123Z",      // space separator
            "2026-11-17",                    // date only
            "2026-11-17T13:03Z",             // minute precision
            "2026-11-17T13:03:22.12aZ",      // non-digit in the fraction
            "",                              // empty
        ] {
            assert!(
                !is_db002_shape(rejected),
                "{rejected:?} must not pass as a DB-002 timestamp"
            );
        }
    }

    /// Why the exact shape matters: two DB-002 values compare
    /// lexicographically exactly as their instants compare, while two
    /// merely-parseable RFC 3339 spellings of one instant do not — the
    /// property range filters over `ts` rely on, pinned as behavior.
    #[test]
    fn db002_values_order_lexicographically_as_instants() {
        assert!("2026-11-17T13:03:22.123Z" < "2026-11-17T13:03:22.124Z");
        assert!("2026-11-17T13:03:22.999Z" < "2026-11-17T13:03:23.000Z");
        assert!("2026-11-17T23:59:59.999Z" < "2026-11-18T00:00:00.000Z");
        // The counter-example that motivates rejecting near misses:
        // the same instant, two spellings, unequal text comparison.
        assert!("2026-11-17T13:03:22Z" != "2026-11-17T13:03:22.000Z");
        assert!("2026-11-17T13:03:22Z" > "2026-11-17T13:03:22.000Z");
    }

    #[test]
    fn now_utc_returns_the_current_time() {
        let before = Timestamp::now();
        let value = now_utc();
        let after = Timestamp::now();

        // Millisecond resolution on both sides: the helper truncates the
        // sub-millisecond digits, so the parsed instant is compared at the
        // same resolution (truncation is monotonic, so `before`'s truncated
        // millisecond never exceeds the value's). The ±1 s window absorbs a
        // system clock step between the reads; it still pins the value to
        // "now", not to any other point in time.
        let parsed: Timestamp = value.parse().expect("timestamp parses as RFC 3339");
        const TOLERANCE_MS: i64 = 1_000;
        assert!(
            parsed.as_millisecond() >= before.as_millisecond() - TOLERANCE_MS
                && parsed.as_millisecond() <= after.as_millisecond() + TOLERANCE_MS,
            "timestamp {value} is not around {before}..{after}"
        );
    }

    #[test]
    fn new_id_is_canonical_uuidv7_text() {
        for _ in 0..1000 {
            let id = new_id();
            let bytes = id.as_bytes();
            assert_eq!(bytes.len(), 36, "id must be 36 characters: {id}");
            for index in [8, 13, 18, 23] {
                assert_eq!(bytes[index], b'-', "byte {index} of {id}");
            }
            for (index, byte) in bytes.iter().enumerate() {
                if matches!(index, 8 | 13 | 18 | 23) {
                    continue;
                }
                assert!(
                    byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase(),
                    "byte {index} of {id} must be a lowercase hex digit"
                );
            }
            assert_eq!(bytes[14], b'7', "version nibble of {id}");
            assert!(
                matches!(bytes[19], b'8' | b'9' | b'a' | b'b'),
                "variant nibble of {id}"
            );
            // A canonical rendering round-trips through the parser.
            assert_eq!(
                Uuid::parse_str(&id).unwrap().to_string(),
                id,
                "id must round-trip through the canonical form"
            );
        }
    }

    #[test]
    fn new_ids_from_one_process_are_strictly_increasing() {
        let ids: Vec<String> = (0..1000).map(|_| new_id()).collect();
        for pair in ids.windows(2) {
            assert!(
                pair[0] < pair[1],
                "{} must sort before {}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn new_ids_stay_ordered_across_a_millisecond_boundary() {
        // The tight loop above completes within one millisecond on any
        // realistic machine, so it exercises the same-millisecond counter
        // path; this companion pins the cross-millisecond path, where the
        // ordering comes from the timestamp bits. `thread::sleep` sleeps at
        // least the requested duration, so a millisecond boundary
        // necessarily passes between the two calls. The outcome is
        // deterministic either way: both regimes are ordered by the same
        // per-process contract (DECISIONS.md D-031).
        let first = new_id();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = new_id();
        assert!(first < second, "{first} must sort before {second}");
    }
}
