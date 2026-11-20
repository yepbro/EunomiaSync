//! The creation-marker line (spec §9.3, ALG-032; D-23, OUT-016…018).
//!
//! Every built content ends with one line recording when it was
//! built — an HTML comment in local time with the time zone (OUT-016,
//! OUT-017, OUT-018). This module owns the line's exact format:
//!
//! ```text
//! <!-- Created by EunomiaSync YYYY-MM-DD HH:MM:SS <IANA name> <offset> -->
//! ```
//!
//! for example, the spec's own example (ALG-032, D-23):
//!
//! ```text
//! <!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->
//! ```
//!
//! When no IANA name is available, `UTC` plus the offset takes the
//! name-and-offset place: `<!-- Created by EunomiaSync 2026-11-17
//! 14:03:22 UTC+01:00 -->` (D-23's "Without an IANA name: `UTC+01:00`"
//! — the compact form replaces both the name and the offset fields, it
//! does not stack after them). The offset keeps its explicit sign in
//! every case, `+00:00` for a zero offset like `-05:00` for a negative
//! one: the template's `±hh:mm` always carries the sign (and RFC 3339
//! reserves `-00:00` for an *unknown* offset, which a built marker
//! never has).
//!
//! # No clock here
//!
//! ALG-032 says the time "is taken once per run, at the start of the
//! write phase, and is the same for all projects" — that capture is
//! [`marker_clock`](crate::marker_clock)'s job, not this function's
//! (it is the crate's one clock read: `jiff`, whose edge extends
//! D-017's allow-list — see there and DECISIONS.md D-104).
//! [`creation_marker_line`] stays a pure formatter: the instant, the
//! zone and the offset are all inputs, crossing the boundary already
//! decomposed into the six civil fields the marker prints, plus the
//! IANA name and the offset in minutes. The spec's
//! `YYYY-MM-DD HH:MM:SS` zero-padding then has exactly one
//! implementation — this one — instead of one per caller.
//!
//! # Boundary
//!
//! What surrounds the line belongs to the file-assembly tasks that
//! consume it: the empty line before it and the trailing line break of
//! ALG-030, scheme B's placement before the closing marker (OUT-016),
//! and the ALG-033 scanner that later finds the line again. None of
//! that changes the line itself, which is why it lives here alone.

/// The local wall-clock time a marker line stamps (spec §9.3, ALG-032).
///
/// The six fields of `YYYY-MM-DD HH:MM:SS`, decomposed — the shape a
/// time library's civil accessors hand out. The fields arrive
/// resolved by one: `marker_clock` produces them from `jiff`
/// (D-104's edge on top of D-017's allow-list); this type itself is a
/// plain record with no clock of its own.
///
/// The field domains are checked by [`creation_marker_line`]; month
/// lengths are not. A day of 31 is accepted in any month — whether
/// April has a 31st is the time library's calendar, not the marker's;
/// the fields arrive from one, so the question never occurs in practice.
///
/// # Examples
///
/// ```
/// use eunomia_core::marker::MarkerTime;
///
/// // The ALG-032 example instant: 2026-11-17 14:03:22 local.
/// let time = MarkerTime {
///     year: 2026,
///     month: 11,
///     day: 17,
///     hour: 14,
///     minute: 3,
///     second: 22,
/// };
/// assert_eq!((time.year, time.month, time.day), (2026, 11, 17));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkerTime {
    /// The year, `1..=9999`, printed as four digits (`YYYY`).
    pub year: i16,
    /// The month, `1..=12`, printed as two digits (`MM`).
    pub month: u8,
    /// The day of the month, `1..=31`, printed as two digits (`DD`).
    pub day: u8,
    /// The hour, `0..=23`, printed as two digits (`HH`).
    pub hour: u8,
    /// The minute, `0..=59`, printed as two digits (`MM`).
    pub minute: u8,
    /// The second, `0..=59`, printed as two digits (`SS`).
    pub second: u8,
}

/// Formats one creation-marker line (spec §9.3, ALG-032; D-23).
///
/// With an IANA name the line carries the name and the offset as two
/// fields; without one, `UTC` and the offset merge into the single
/// `UTC±hh:mm` form of D-23. The offset is whole minutes east of UTC,
/// always printed with its sign and as `hh:mm` — `+01:00`, `-05:00`,
/// `+00:00`, `+05:45`. No clock is read: `time`, `iana_zone` and
/// `offset_minutes` are the whole truth (ALG-032 takes the time "once
/// per run" — the caller's job).
///
/// The line is exactly the comment; the empty line before it, the
/// trailing line break and the scheme placement (ALG-030, ALG-031,
/// OUT-016) belong to the caller, and no line break is appended here.
///
/// # Panics
///
/// Panics if a field leaves its documented domain (a [`MarkerTime`]
/// field or an empty IANA name), or if the offset lies outside
/// `±24:00` in minutes. These are broken-invariant panics in the
/// crate's usual posture, not data conditions: the fields come from
/// a time library's civil accessors, which never produce out-of-range
/// values, and every real zone sits inside `±14:00` — the bound only
/// guards against synthetic values (`jiff`'s own `Offset` domain
/// reaches `±25:59:59`, so an offset handed through unchecked could
/// cross it).
///
/// # Examples
///
/// ```
/// use eunomia_core::marker::{creation_marker_line, MarkerTime};
///
/// let time = MarkerTime {
///     year: 2026,
///     month: 11,
///     day: 17,
///     hour: 14,
///     minute: 3,
///     second: 22,
/// };
///
/// // The ALG-032 / D-23 example, verbatim.
/// assert_eq!(
///     creation_marker_line(time, Some("Europe/Prague"), 60),
///     "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->",
/// );
///
/// // Without an IANA name (D-23): UTC + offset as one field.
/// assert_eq!(
///     creation_marker_line(time, None, 60),
///     "<!-- Created by EunomiaSync 2026-11-17 14:03:22 UTC+01:00 -->",
/// );
/// ```
#[must_use]
pub fn creation_marker_line(
    time: MarkerTime,
    iana_zone: Option<&str>,
    offset_minutes: i32,
) -> String {
    let MarkerTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
    } = time;

    assert!(
        (1..=9999).contains(&year),
        "marker year {year} is outside 1..=9999"
    );
    assert!(
        (1..=12).contains(&month),
        "marker month {month} is outside 1..=12"
    );
    assert!(
        (1..=31).contains(&day),
        "marker day {day} is outside 1..=31"
    );
    assert!(
        (0..=23).contains(&hour),
        "marker hour {hour} is outside 0..=23"
    );
    assert!(
        (0..=59).contains(&minute),
        "marker minute {minute} is outside 0..=59"
    );
    assert!(
        (0..=59).contains(&second),
        "marker second {second} is outside 0..=59"
    );
    if let Some(name) = iana_zone {
        assert!(
            !name.is_empty(),
            "an IANA time zone name must not be empty — pass None when the zone is unknown"
        );
    }
    assert!(
        offset_minutes.unsigned_abs() <= 24 * 60,
        "marker offset {offset_minutes} minutes lies outside ±24:00"
    );

    let offset = format_offset(offset_minutes);
    let zone = match iana_zone {
        Some(name) => format!("{name} {offset}"),
        None => format!("UTC{offset}"),
    };

    format!(
        "<!-- Created by EunomiaSync {year:04}-{month:02}-{day:02} \
         {hour:02}:{minute:02}:{second:02} {zone} -->"
    )
}

/// Renders the offset as ALG-032's `±hh:mm`: explicit sign, two-digit
/// hours and minutes, colon between — `+01:00`, `-05:00`, `+00:00`,
/// `+05:45`.
///
/// Zero keeps the plus sign (RFC 3339 reserves `-00:00` for an unknown
/// offset, which a stamped marker never has); the parameter carries
/// whole minutes, so sub-minute offsets are inexpressible by design —
/// and no zone of the present day has one anyway.
fn format_offset(offset_minutes: i32) -> String {
    let sign = if offset_minutes < 0 { '-' } else { '+' };
    let magnitude = offset_minutes.unsigned_abs();
    format!("{sign}{:02}:{:02}", magnitude / 60, magnitude % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ALG-032 / D-23 example instant: 2026-11-17 14:03:22.
    fn spec_time() -> MarkerTime {
        MarkerTime {
            year: 2026,
            month: 11,
            day: 17,
            hour: 14,
            minute: 3,
            second: 22,
        }
    }

    // ---- ALG-032 / D-23: the spec oracles, verbatim ---------------------

    #[test]
    fn spec_example_with_iana_name_is_verbatim() {
        // Hardcoded from ALG-032 / D-23, independent of the
        // implementation: Europe/Prague, standard time, +01:00.
        assert_eq!(
            creation_marker_line(spec_time(), Some("Europe/Prague"), 60),
            "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->"
        );
    }

    #[test]
    fn spec_example_without_iana_name_uses_the_utc_form() {
        // D-23: "Without an IANA name: `UTC+01:00`" — UTC and the
        // offset merge into one field; no bare offset follows it.
        assert_eq!(
            creation_marker_line(spec_time(), None, 60),
            "<!-- Created by EunomiaSync 2026-11-17 14:03:22 UTC+01:00 -->"
        );
    }

    // ---- offsets ---------------------------------------------------------

    #[test]
    fn negative_offset_keeps_the_minus_sign() {
        assert_eq!(
            creation_marker_line(spec_time(), Some("America/New_York"), -5 * 60),
            "<!-- Created by EunomiaSync 2026-11-17 14:03:22 America/New_York -05:00 -->"
        );
    }

    #[test]
    fn zero_offset_prints_plus_zero_zero() {
        // Europe/London on winter time: a real named zone at +00:00.
        assert_eq!(
            creation_marker_line(spec_time(), Some("Europe/London"), 0),
            "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/London +00:00 -->"
        );
        // And the UTC form at zero offset (D-23 wording says
        // `UTC+01:00`; the zero case follows the same template).
        assert_eq!(
            creation_marker_line(spec_time(), None, 0),
            "<!-- Created by EunomiaSync 2026-11-17 14:03:22 UTC+00:00 -->"
        );
    }

    #[test]
    fn positive_offsets_split_into_hours_and_minutes() {
        // Zones off the whole hour: Kolkata +05:30, Kathmandu +05:45 —
        // the hour/minute split, not decimal or total minutes.
        assert_eq!(
            creation_marker_line(spec_time(), Some("Asia/Kolkata"), 5 * 60 + 30),
            "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Asia/Kolkata +05:30 -->"
        );
        assert_eq!(
            creation_marker_line(spec_time(), Some("Asia/Kathmandu"), 5 * 60 + 45),
            "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Asia/Kathmandu +05:45 -->"
        );
    }

    #[test]
    fn negative_non_hour_offset_is_hours_and_minutes() {
        // -09:30 (Tahiti / Marquesas style offsets): sign and split.
        assert_eq!(
            creation_marker_line(spec_time(), Some("Pacific/Marquesas"), -(9 * 60 + 30)),
            "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Pacific/Marquesas -09:30 -->"
        );
    }

    // ---- field formatting --------------------------------------------------

    #[test]
    fn single_digit_fields_are_zero_padded() {
        // Every field below ten needs its leading zero; the year
        // prints four digits (a year > 9999 does not exist here).
        let time = MarkerTime {
            year: 7,
            month: 1,
            day: 5,
            hour: 9,
            minute: 3,
            second: 7,
        };
        assert_eq!(
            creation_marker_line(time, Some("Europe/Prague"), 60),
            "<!-- Created by EunomiaSync 0007-01-05 09:03:07 Europe/Prague +01:00 -->"
        );
    }

    #[test]
    fn the_line_is_self_contained_with_no_line_break() {
        // The empty line before the marker and the trailing break are
        // ALG-030's, not this function's (the module boundary).
        let line = creation_marker_line(spec_time(), None, 60);
        assert!(line.starts_with("<!-- Created by EunomiaSync "));
        assert!(line.ends_with(" -->"));
        assert!(!line.contains('\n'));
        assert!(!line.contains('\r'));
        // Exactly one space between every field, nowhere two.
        assert!(!line.contains("  "));
    }

    #[test]
    fn every_space_of_the_template_is_in_place() {
        // The template character by character: comment open, the fixed
        // phrase, date, one space, time, one space, zone field, one
        // space, comment close — the ALG-032 reading the scanner of
        // ALG-033 will rely on.
        let line = creation_marker_line(spec_time(), Some("Europe/Prague"), 60);
        assert_eq!(
            line,
            "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->"
        );
        // The pieces the scanner matches on, in order.
        let mut fields = line.split(' ');
        assert_eq!(fields.next(), Some("<!--"));
        assert_eq!(fields.next(), Some("Created"));
        assert_eq!(fields.next(), Some("by"));
        assert_eq!(fields.next(), Some("EunomiaSync"));
        assert_eq!(fields.next(), Some("2026-11-17"));
        assert_eq!(fields.next(), Some("14:03:22"));
        assert_eq!(fields.next(), Some("Europe/Prague"));
        assert_eq!(fields.next(), Some("+01:00"));
        assert_eq!(fields.next(), Some("-->"));
        assert_eq!(fields.next(), None);
    }

    // ---- broken invariants panic -------------------------------------------

    #[test]
    #[should_panic(expected = "marker year 10000 is outside 1..=9999")]
    fn a_year_out_of_domain_panics() {
        let _ = creation_marker_line(
            MarkerTime {
                year: 10_000,
                ..spec_time()
            },
            None,
            0,
        );
    }

    #[test]
    #[should_panic(expected = "marker minute 60 is outside 0..=59")]
    fn a_minute_out_of_domain_panics() {
        let _ = creation_marker_line(
            MarkerTime {
                minute: 60,
                ..spec_time()
            },
            None,
            0,
        );
    }

    #[test]
    #[should_panic(expected = "marker month 13 is outside 1..=12")]
    fn a_month_out_of_domain_panics() {
        let _ = creation_marker_line(
            MarkerTime {
                month: 13,
                ..spec_time()
            },
            None,
            0,
        );
    }

    #[test]
    #[should_panic(expected = "marker hour 24 is outside 0..=23")]
    fn an_hour_out_of_domain_panics() {
        let _ = creation_marker_line(
            MarkerTime {
                hour: 24,
                ..spec_time()
            },
            None,
            0,
        );
    }

    #[test]
    #[should_panic(expected = "marker second 60 is outside 0..=59")]
    fn a_sixty_first_second_panics() {
        let _ = creation_marker_line(
            MarkerTime {
                second: 60,
                ..spec_time()
            },
            None,
            0,
        );
    }

    #[test]
    #[should_panic(expected = "marker day 0 is outside 1..=31")]
    fn a_day_zero_panics() {
        let _ = creation_marker_line(
            MarkerTime {
                day: 0,
                ..spec_time()
            },
            None,
            0,
        );
    }

    #[test]
    #[should_panic(expected = "must not be empty")]
    fn an_empty_zone_name_panics_instead_of_double_spacing() {
        let _ = creation_marker_line(spec_time(), Some(""), 0);
    }

    #[test]
    #[should_panic(expected = "outside ±24:00")]
    fn an_offset_beyond_twenty_four_hours_panics() {
        let _ = creation_marker_line(spec_time(), None, 24 * 60 + 1);
    }

    #[test]
    #[should_panic(expected = "outside ±24:00")]
    fn a_hugely_negative_offset_panics() {
        let _ = creation_marker_line(spec_time(), None, -(24 * 60 + 1));
    }

    #[test]
    fn the_extreme_allowed_offsets_format() {
        // ±24:00 exactly is inside the boundary (and outside every
        // real zone — IANA tops out at ±14:00 — hence the placeholder
        // names: the line formats the name verbatim and never checks
        // it against the offset; that correspondence is the caller's
        // time library's truth).
        assert_eq!(
            creation_marker_line(spec_time(), Some("Example/Edge"), 24 * 60),
            "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Example/Edge +24:00 -->"
        );
        assert_eq!(
            creation_marker_line(spec_time(), Some("Example/Edge"), -24 * 60),
            "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Example/Edge -24:00 -->"
        );
    }
}
