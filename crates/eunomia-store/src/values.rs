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
//! The `uuid` and `chrono` dependencies behind this module were added per
//! DECISIONS.md D-031.

use chrono::{SecondsFormat, Utc};
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
/// (not rounded): [`SecondsFormat::Millis`] formats `nanos / 1_000_000`.
pub fn now_utc() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
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
    fn now_utc_returns_the_current_time() {
        let before = Utc::now();
        let value = now_utc();
        let after = Utc::now();

        // Millisecond resolution on both sides: the helper truncates the
        // sub-millisecond digits, so the parsed instant is compared at the
        // same resolution (truncation is monotonic, so `before`'s truncated
        // millisecond never exceeds the value's). The ±1 s window absorbs a
        // system clock step between the reads; it still pins the value to
        // "now", not to any other point in time.
        let parsed = chrono::DateTime::parse_from_rfc3339(&value)
            .expect("timestamp parses as RFC 3339")
            .with_timezone(&Utc);
        const TOLERANCE_MS: i64 = 1_000;
        assert!(
            parsed.timestamp_millis() >= before.timestamp_millis() - TOLERANCE_MS
                && parsed.timestamp_millis() <= after.timestamp_millis() + TOLERANCE_MS,
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
