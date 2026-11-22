//! The run clock behind the creation marker (spec §9.3, ALG-032; D-23).
//!
//! ALG-032 says the marker's time "is taken once per run, at the start of
//! the write phase, and is the same for all projects". This module is
//! that taking: it resolves one instant against one time zone and hands
//! the result to [`marker::creation_marker_line`], which owns the line's
//! exact format (D-061). The split is deliberate — the formatter stays
//! pure and clock-free, this module is the only place the app touches a
//! clock for the marker, so "same marker for every file of the run" is
//! a type property, not a discipline of every caller: capture one
//! [`MarkerClock`], ask it for a line per file, get identical lines.
//!
//! # Where the zone comes from
//!
//! [`now_marker_clock`] asks `jiff` for the system time zone: the `TZ`
//! environment variable first, `/etc/localtime` (Unix) or the registry
//! (Windows) behind it — jiff 1.x with the crate features enabled for
//! this module (`tz-system`, `tzdb-zoneinfo`, `tzdb-bundle-platform`;
//! DECISIONS.md D-035's time-library ruling and the entry it predicted).
//! The three zone shapes the marker cares about all fall out of
//! `TimeZone::iana_name`:
//!
//! - a named IANA zone (`Europe/Prague`) → the marker prints
//!   `Europe/Prague +01:00`;
//! - a zone without an IANA name — a fixed offset, a POSIX `TZ` rule,
//!   or an unnamed `/etc/localtime` copy — → D-23's compact form
//!   `UTC±hh:mm`;
//! - jiff's `Etc/Unknown` fallback, when system detection fails
//!   outright (the zone then behaves like UTC), reports no name, so the
//!   marker honestly degrades to `UTC+00:00` instead of claiming a zone
//!   it could not find.
//!
//! `UTC` itself is an IANA identifier (`TimeZone::UTC.iana_name()` is
//! `Some("UTC")`), so a machine actually set to UTC stamps the
//! name-and-offset form `UTC +00:00`, not the fallback form — the
//! fallback is reserved for a zone whose name is *unknown*, which is
//! D-23's wording. jiff reports the identifier a zone was loaded
//! under, case-normalized: an `/etc/localtime` symlinked to `Etc/UTC`
//! stamps `Etc/UTC`, because aliases are valid identifiers too and
//! resolving them to primary names is not the marker's business.
//!
//! # Determinism
//!
//! The system clock and the system zone are not test inputs. Every
//! behavior below is reached through [`MarkerClock::from_instant`], a
//! pure seam over a given instant and zone; [`now_marker_clock`] is
//! nothing but `from_instant(Timestamp::now(), TimeZone::system())`.
//! Tests fix instants (as RFC 3339 strings, with the epoch noted) and
//! zones, and read the same lines production reads.

use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::marker::MarkerTime;
use crate::marker::creation_marker_line;

/// The marker's "now": one instant resolved against one zone (ALG-032).
///
/// The value the write phase captures once and reuses for every file of
/// the run. It carries the three things the marker line prints,
/// decomposed at capture time: the local wall-clock fields
/// ([`MarkerTime`]), the IANA name of the zone when it has one, and the
/// zone's offset from UTC at that instant, in whole minutes east. The
/// fields are private on purpose: they are one observation, not three
/// independent values — a caller who could set them separately could
/// stamp a time with one zone's wall clock and another zone's offset.
///
/// [`Clone`] is cheap and the value is immutable, so "the same for all
/// projects" needs no sharing: copies stay identical.
///
/// # Examples
///
/// ```
/// use eunomia_core::marker_clock::MarkerClock;
/// use jiff::Timestamp;
/// use jiff::tz::TimeZone;
///
/// // The ALG-032 example instant: 2026-11-17 14:03:22 Europe/Prague.
/// let instant: Timestamp = "2026-11-17T13:03:22Z".parse().unwrap();
/// let clock = MarkerClock::from_instant(instant, TimeZone::get("Europe/Prague").unwrap());
/// assert_eq!(clock.iana_zone(), Some("Europe/Prague"));
/// assert_eq!(clock.offset_minutes(), 60);
/// assert_eq!(
///     clock.marker_line(),
///     "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->",
/// );
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkerClock {
    /// The local wall-clock fields of the instant in the zone.
    time: MarkerTime,
    /// The zone's IANA name as jiff reports it for the loaded zone
    /// (case-normalized; aliases like `Etc/UTC` stay as loaded),
    /// `None` when it has none (fixed offset, POSIX rule, unnamed
    /// TZif, `Etc/Unknown`).
    iana_zone: Option<String>,
    /// The zone's offset at the instant, whole minutes east of UTC.
    offset_minutes: i32,
}

impl MarkerClock {
    /// Resolves one instant against one zone into the marker's fields.
    ///
    /// The seam every test and [`now_marker_clock`] go through: the
    /// wall-clock fields come from the instant rendered *in this zone*
    /// (so a DST boundary is honored — the offset is the one in force
    /// at the instant, not the zone's standard-time offset), the name
    /// from the zone, the offset from the rendering. The instant's
    /// sub-second part does not survive: `MarkerTime` is second-granular
    /// and the marker template has no fraction field.
    ///
    /// A zone offset that is not a whole number of minutes — only a
    /// POSIX `TZ` rule or a historical pre-railway LMT can produce one
    /// today — is truncated toward zero to the minute, never rounded:
    /// the marker template cannot say more, and no present-day zone
    /// offsets by a fraction of a minute at run time anyway.
    ///
    /// # Panics
    ///
    /// This constructor cannot panic; [`Self::marker_line`] can; see
    /// there.
    #[must_use]
    pub fn from_instant(instant: Timestamp, zone: TimeZone) -> Self {
        let local = instant.to_zoned_datetime(zone.clone());
        let time = MarkerTime {
            year: local.year(),
            month: local
                .month()
                .try_into()
                .expect("jiff months are 1..=12, inside MarkerTime's domain"),
            day: local
                .day()
                .try_into()
                .expect("jiff days are 1..=31, inside MarkerTime's domain"),
            hour: local
                .hour()
                .try_into()
                .expect("jiff hours are 0..=23, inside MarkerTime's domain"),
            minute: local
                .minute()
                .try_into()
                .expect("jiff minutes are 0..=59, inside MarkerTime's domain"),
            second: local
                .second()
                .try_into()
                .expect("jiff seconds are 0..=59, inside MarkerTime's domain"),
        };
        let iana_zone = zone.iana_name().map(str::to_owned);
        // Truncating division: i32 division in Rust cuts toward zero,
        // which is the documented truncation, sign included.
        let offset_minutes = local.offset().seconds() / 60;
        Self {
            time,
            iana_zone,
            offset_minutes,
        }
    }

    /// The captured local wall-clock time.
    #[must_use]
    pub fn time(&self) -> MarkerTime {
        self.time
    }

    /// The zone's IANA name, `None` when the zone has none — the case
    /// D-23's `UTC±hh:mm` fallback exists for.
    #[must_use]
    pub fn iana_zone(&self) -> Option<&str> {
        self.iana_zone.as_deref()
    }

    /// The zone's offset at the captured instant, whole minutes east of
    /// UTC (`+60` is `+01:00`, `-300` is `-05:00`).
    #[must_use]
    pub fn offset_minutes(&self) -> i32 {
        self.offset_minutes
    }

    /// Formats the marker line for this capture via
    /// [`creation_marker_line`] — the single owner of the template
    /// (D-061). Calling it any number of times returns the same line:
    /// the capture is immutable, which is ALG-032's "same for all
    /// projects" made concrete.
    ///
    /// # Panics
    ///
    /// Inherits [`creation_marker_line`]'s broken-invariant panics. Of
    /// those only the `±24:00` offset bound is reachable through this
    /// type: real IANA zones stay within `±14:00`, but jiff's
    /// fixed-offset and POSIX-rule zones reach `±25:59` — and a POSIX
    /// rule is not synthetic in the relevant sense, since `TZ` can
    /// hand one to the system zone. [`now_marker_clock`] guards that
    /// path (an out-of-domain capture degrades to the nameless UTC
    /// form instead of panicking), so the panic is reachable only
    /// through `from_instant` called with a zone beyond the marker's
    /// domain — marker.rs's posture that an offset outside the
    /// template's own domain is a broken invariant, not a data
    /// condition.
    #[must_use]
    pub fn marker_line(&self) -> String {
        creation_marker_line(self.time, self.iana_zone.as_deref(), self.offset_minutes)
    }
}

/// Captures the marker clock for one run: the current instant in the
/// system time zone (ALG-032's "taken once per run, at the start of
/// the write phase"). That start is §9.4's phase 3, the write — not
/// phase 0, which builds the content of all projects but takes no
/// marker, and not the preflight, which cannot know the line
/// (D-133): the capture belongs there, before the write phase's
/// first write, so every file of the run stamps one observation.
///
/// The write phase asks for this capture at most once (its clock
/// seam is `FnOnce`, and a run with nothing to write captures
/// nothing at all), then formats one marker line from the returned
/// [`MarkerClock`] and hands the same line to every project's
/// assembly; the files of the run carry one marker, byte for byte.
/// When the system zone cannot be detected, jiff's `Etc/Unknown`
/// stand-in keeps this infallible and nameless, so the marker
/// degrades to D-23's `UTC+00:00` form rather than failing the run
/// over a stamp. The same degradation — not a
/// panic — applies when a `TZ` POSIX rule names an offset beyond the
/// marker's own `±24:00` domain (the only environment that can; see
/// [`capture_run_clock`]).
///
/// # Examples
///
/// ```
/// use eunomia_core::marker_clock::now_marker_clock;
///
/// // One capture serves the whole run: a well-formed marker line,
/// // identical on every ask.
/// let clock = now_marker_clock();
/// let line = clock.marker_line();
/// assert_eq!(clock.marker_line(), line);
/// assert!(line.starts_with("<!-- Created by EunomiaSync "));
/// assert!(line.ends_with(" -->"));
/// ```
#[must_use]
pub fn now_marker_clock() -> MarkerClock {
    capture_run_clock(Timestamp::now(), TimeZone::system())
}

/// The seam behind [`now_marker_clock`]: the capture, plus the one
/// guard the run entry point owes its environment. A system zone whose
/// offset leaves the marker's `±24:00` domain — only a POSIX `TZ` rule
/// can name one (`TZ=AAA-24:59:59BBB,…`, a day and a minute east of
/// UTC; real zones stop at `±14:00`) — would make
/// [`MarkerClock::marker_line`] panic; the run instead re-resolves the
/// *same* instant (the clock is read once, never twice) against
/// jiff's nameless `Etc/Unknown`, degrading the stamp to the honest
/// `UTC+00:00` form with a wall clock that agrees with its offset.
fn capture_run_clock(instant: Timestamp, zone: TimeZone) -> MarkerClock {
    let clock = MarkerClock::from_instant(instant, zone);
    if clock.offset_minutes.unsigned_abs() <= 24 * 60 {
        clock
    } else {
        MarkerClock::from_instant(instant, TimeZone::unknown())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::tz::Offset;

    /// Parses an RFC 3339 instant, so tests read as instants, not as
    /// epoch counters.
    fn instant(rfc3339: &str) -> Timestamp {
        rfc3339.parse().expect("test fixtures are valid RFC 3339")
    }

    // ---- named IANA zones --------------------------------------------------

    #[test]
    fn spec_example_instant_in_prague_is_the_spec_line_verbatim() {
        // 2026-11-17T13:03:22Z is 14:03:22 in Prague (winter, +01:00):
        // the ALG-032 / D-23 example, reached through the clock seam
        // instead of hardcoded fields — epoch 1794920602.
        let clock = MarkerClock::from_instant(
            instant("2026-11-17T13:03:22Z"),
            TimeZone::get("Europe/Prague").expect("Europe/Prague is in the tzdb"),
        );
        assert_eq!(
            clock.marker_line(),
            "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->"
        );
    }

    #[test]
    fn a_named_zone_reports_the_name_it_was_loaded_under() {
        // jiff reports the identifier the zone was loaded under,
        // case-normalized; aliases stay aliases (the Etc/UTC case
        // below pins that), and the marker prints that name as-is —
        // ALG-032 asks for `<IANA name>`, and resolving aliases to
        // primary names is not the marker's business.
        let clock = MarkerClock::from_instant(
            instant("2026-06-21T12:00:00Z"),
            TimeZone::get("Europe/Berlin").expect("Europe/Berlin is in the tzdb"),
        );
        assert_eq!(clock.iana_zone(), Some("Europe/Berlin"));
        // Summer: Berlin is on CEST, +02:00 — the offset at the
        // instant, not the zone's standard-time +01:00.
        assert_eq!(clock.offset_minutes(), 120);
        assert_eq!(clock.time().hour, 14);
    }

    #[test]
    fn a_machine_set_to_utc_prints_the_name_form_not_the_fallback() {
        // "UTC" is an IANA identifier: jiff reports it as a name, and
        // the name-and-offset template applies. D-23's UTC±hh:mm form
        // is for a zone whose name is unknown, not for UTC itself.
        let utc = MarkerClock::from_instant(instant("2026-11-17T13:03:22Z"), TimeZone::UTC);
        assert_eq!(utc.iana_zone(), Some("UTC"));
        assert_eq!(
            utc.marker_line(),
            "<!-- Created by EunomiaSync 2026-11-17 13:03:22 UTC +00:00 -->"
        );
        // Etc/UTC — the alias many distros point /etc/localtime at —
        // is a name too, kept as loaded, so the name form applies
        // there as well.
        let etc_utc = MarkerClock::from_instant(
            instant("2026-11-17T13:03:22Z"),
            TimeZone::get("Etc/UTC").expect("Etc/UTC is in the tzdb"),
        );
        assert_eq!(etc_utc.iana_zone(), Some("Etc/UTC"));
        assert_eq!(
            etc_utc.marker_line(),
            "<!-- Created by EunomiaSync 2026-11-17 13:03:22 Etc/UTC +00:00 -->"
        );
    }

    // ---- zones without an IANA name: D-23's UTC±hh:mm -----------------------

    #[test]
    fn a_fixed_offset_zone_without_a_name_prints_the_utc_form() {
        // TimeZone::fixed-like zones carry no name; D-23's compact
        // form replaces the whole name-and-offset pair.
        let clock = MarkerClock::from_instant(
            instant("2026-11-17T13:03:22Z"),
            Offset::constant(1).to_time_zone(),
        );
        assert_eq!(clock.iana_zone(), None);
        assert_eq!(clock.offset_minutes(), 60);
        assert_eq!(
            clock.marker_line(),
            "<!-- Created by EunomiaSync 2026-11-17 14:03:22 UTC+01:00 -->"
        );
    }

    #[test]
    fn a_fixed_negative_offset_keeps_the_sign_in_the_utc_form() {
        // Offset::constant(-5) is UTC-5: the wall clock moves back and
        // the compact form carries the minus.
        let clock = MarkerClock::from_instant(
            instant("2026-11-17T13:03:22Z"),
            Offset::constant(-5).to_time_zone(),
        );
        assert_eq!(clock.offset_minutes(), -300);
        assert_eq!(
            clock.marker_line(),
            "<!-- Created by EunomiaSync 2026-11-17 08:03:22 UTC-05:00 -->"
        );
    }

    #[test]
    fn a_posix_tz_rule_without_a_name_prints_the_utc_form() {
        // The realistic "TZ is set but names no zone" case: a POSIX
        // rule (here Central-European clocks, exactly the shape a
        // TZ=CET-1CEST,M3.5.0,M10.5.0/3 environment would give) has
        // DST transitions and no IANA name.
        let zone = TimeZone::posix("CET-1CEST,M3.5.0,M10.5.0/3")
            .expect("a well-formed POSIX TZ rule parses");
        assert_eq!(zone.iana_name(), None);
        // Winter instant: +01:00 (CET).
        let winter = MarkerClock::from_instant(instant("2026-11-17T13:03:22Z"), zone.clone());
        assert_eq!(winter.offset_minutes(), 60);
        assert_eq!(
            winter.marker_line(),
            "<!-- Created by EunomiaSync 2026-11-17 14:03:22 UTC+01:00 -->"
        );
        // Summer instant: the rule's DST leg, +02:00 (CEST) — the
        // offset follows the instant even without a name.
        let summer = MarkerClock::from_instant(instant("2026-06-21T12:00:00Z"), zone);
        assert_eq!(summer.offset_minutes(), 120);
        assert_eq!(
            summer.marker_line(),
            "<!-- Created by EunomiaSync 2026-06-21 14:00:00 UTC+02:00 -->"
        );
    }

    #[test]
    fn a_sub_minute_zone_offset_truncates_toward_zero() {
        // +05:45:30 (a hypothetical sub-minute offset — constructible
        // via a fixed zone) prints +05:45 with the truncated wall
        // clock 18:48:52 (13:03:22 + 05:45:30), and -05:45:30 prints
        // -05:45 with 07:17:52: truncation, not rounding, on both
        // sides of zero — the full line, so the wall clock and the
        // offset field are pinned together.
        let east = MarkerClock::from_instant(
            instant("2026-11-17T13:03:22Z"),
            Offset::from_seconds(5 * 3600 + 45 * 60 + 30)
                .expect("inside jiff's offset domain")
                .to_time_zone(),
        );
        assert_eq!(east.offset_minutes(), 345);
        assert_eq!(
            east.marker_line(),
            "<!-- Created by EunomiaSync 2026-11-17 18:48:52 UTC+05:45 -->"
        );

        let west = MarkerClock::from_instant(
            instant("2026-11-17T13:03:22Z"),
            Offset::from_seconds(-(5 * 3600 + 45 * 60 + 30))
                .expect("inside jiff's offset domain")
                .to_time_zone(),
        );
        assert_eq!(west.offset_minutes(), -345);
        assert_eq!(
            west.marker_line(),
            "<!-- Created by EunomiaSync 2026-11-17 07:17:52 UTC-05:45 -->"
        );
    }

    #[test]
    fn jiffs_unknown_fallback_is_nameless_and_utc() {
        // TimeZone::system() degrades to Etc/Unknown when detection
        // fails; the clock must degrade with it — no name (hence the
        // UTC form), zero offset, wall clock equal to UTC.
        let clock = MarkerClock::from_instant(instant("2026-11-17T13:03:22Z"), TimeZone::unknown());
        assert_eq!(clock.iana_zone(), None);
        assert_eq!(clock.offset_minutes(), 0);
        assert_eq!(
            clock.marker_line(),
            "<!-- Created by EunomiaSync 2026-11-17 13:03:22 UTC+00:00 -->"
        );
    }

    // ---- DST boundaries -----------------------------------------------------

    #[test]
    fn the_fall_back_transition_is_stamped_by_the_offset_in_force() {
        // Europe/Berlin, 2026-10-25: 03:00 CEST becomes 02:00 CET at
        // 01:00 UTC. One minute-pair around it, both rendered as
        // 02:30 wall time — the offset is the only thing that tells
        // them apart, and it must.
        let before = MarkerClock::from_instant(
            instant("2026-10-25T00:30:00Z"),
            TimeZone::get("Europe/Berlin").expect("Europe/Berlin is in the tzdb"),
        );
        assert_eq!(
            before.marker_line(),
            "<!-- Created by EunomiaSync 2026-10-25 02:30:00 Europe/Berlin +02:00 -->"
        );

        let after = MarkerClock::from_instant(
            instant("2026-10-25T01:30:00Z"),
            TimeZone::get("Europe/Berlin").expect("Europe/Berlin is in the tzdb"),
        );
        // The same wall time 02:30, one hour of offsets apart.
        assert_eq!(after.time(), before.time());
        assert_eq!(
            after.marker_line(),
            "<!-- Created by EunomiaSync 2026-10-25 02:30:00 Europe/Berlin +01:00 -->"
        );
    }

    #[test]
    fn the_spring_forward_transition_skips_the_wall_hour() {
        // Europe/Berlin, 2026-03-29: 02:00 CET becomes 03:00 CEST at
        // 01:00 UTC; wall time jumps 01:59 → 03:01 for one minute
        // either side, and the offset flips with it.
        let before = MarkerClock::from_instant(
            instant("2026-03-29T00:59:00Z"),
            TimeZone::get("Europe/Berlin").expect("Europe/Berlin is in the tzdb"),
        );
        assert_eq!(
            before.marker_line(),
            "<!-- Created by EunomiaSync 2026-03-29 01:59:00 Europe/Berlin +01:00 -->"
        );

        let after = MarkerClock::from_instant(
            instant("2026-03-29T01:01:00Z"),
            TimeZone::get("Europe/Berlin").expect("Europe/Berlin is in the tzdb"),
        );
        assert_eq!(
            after.marker_line(),
            "<!-- Created by EunomiaSync 2026-03-29 03:01:00 Europe/Berlin +02:00 -->"
        );
    }

    // ---- one capture, one marker for the whole run ---------------------------

    #[test]
    fn one_capture_gives_every_file_of_the_run_the_same_line() {
        // ALG-032's acceptance criterion at this API's level. That
        // every ask is identical is structural (an immutable capture),
        // so the falsifiable core of the criterion is what the
        // identity is made of: the line must be exactly
        // creation_marker_line over the three published fields, for
        // every zone shape the clock can capture — if marker_line()
        // ever stops deriving from the accessors (a cache, formatting
        // of its own), the files of a run could diverge while an
        // identity-only loop stayed green. (The across-a-real-run
        // oracle — many files, one capture — belongs to the
        // write-phase consumer that captures once and hands every
        // file the one line.)
        let named = MarkerClock::from_instant(
            instant("2026-11-17T13:03:22Z"),
            TimeZone::get("Europe/Prague").expect("Europe/Prague is in the tzdb"),
        );
        let first = named.marker_line();
        // every ask of the same capture: same answer, through a
        // clone too — the write phase captures once, then hands the
        // one formatted line to every file.
        assert_eq!(named.marker_line(), first);
        assert_eq!(named.clone().marker_line(), first);

        for clock in [
            named,
            MarkerClock::from_instant(instant("2026-11-17T13:03:22Z"), TimeZone::UTC),
            MarkerClock::from_instant(
                instant("2026-11-17T13:03:22Z"),
                Offset::constant(-5).to_time_zone(),
            ),
            MarkerClock::from_instant(
                instant("2026-11-17T13:03:22Z"),
                TimeZone::posix("CET-1CEST,M3.5.0,M10.5.0/3")
                    .expect("a well-formed POSIX TZ rule parses"),
            ),
            MarkerClock::from_instant(instant("2026-11-17T13:03:22Z"), TimeZone::unknown()),
        ] {
            assert_eq!(
                clock.marker_line(),
                creation_marker_line(clock.time(), clock.iana_zone(), clock.offset_minutes()),
                "marker_line stopped delegating for {clock:?}"
            );
        }
    }

    #[test]
    fn two_captures_of_one_instant_and_zone_agree() {
        // The seam is deterministic: the same inputs produce equal
        // clocks (the system clock is never consulted).
        let a = MarkerClock::from_instant(
            instant("2026-10-25T01:30:00Z"),
            TimeZone::get("Europe/Berlin").expect("Europe/Berlin is in the tzdb"),
        );
        let b = MarkerClock::from_instant(
            instant("2026-10-25T01:30:00Z"),
            TimeZone::get("Europe/Berlin").expect("Europe/Berlin is in the tzdb"),
        );
        assert_eq!(a, b);
        assert_eq!(a.marker_line(), b.marker_line());
    }

    // ---- now_marker_clock ---------------------------------------------------

    #[test]
    fn now_marker_clock_returns_a_well_formed_line_consistently() {
        // The real clock cannot be pinned to an oracle, so the test
        // holds it to the template ALG-033's scanner relies on and to
        // its own consistency: the line always parses into the fixed
        // comment phrase, a date, a time and a zone field.
        let clock = now_marker_clock();
        let line = clock.marker_line();
        assert_eq!(clock.marker_line(), line);

        assert!(line.starts_with("<!-- Created by EunomiaSync "));
        assert!(line.ends_with(" -->"));
        assert!(!line.contains("  "));
        let mut fields = line.split(' ');
        assert_eq!(fields.next(), Some("<!--"));
        assert_eq!(fields.next(), Some("Created"));
        assert_eq!(fields.next(), Some("by"));
        assert_eq!(fields.next(), Some("EunomiaSync"));
        let date = fields.next().expect("a date field");
        let time = fields.next().expect("a time field");
        // date and time keep ALG-032's fixed widths.
        assert_eq!(date.len(), 10);
        assert_eq!(date.as_bytes()[4], b'-');
        assert_eq!(time.len(), 8);
        assert_eq!(time.as_bytes()[2], b':');
        // the zone is a name+offset pair or the UTC±hh:mm form.
        if let Some(name) = clock.iana_zone() {
            assert_eq!(fields.next(), Some(name));
            let offset = fields.next().expect("an offset field");
            assert!(offset.starts_with('+') || offset.starts_with('-'));
        } else {
            let zone = fields.next().expect("a zone field");
            assert!(
                zone.starts_with("UTC+") || zone.starts_with("UTC-"),
                "zone was {zone}"
            );
        }
        assert_eq!(fields.next(), Some("-->"));
        assert_eq!(fields.next(), None);
        // the offset accessor and the line agree in shape: within the
        // marker's domain, sign explicit either way.
        let offset = clock.offset_minutes();
        assert!(offset.unsigned_abs() <= 24 * 60);
    }

    // ---- the boundary marker.rs pins ----------------------------------------

    #[test]
    #[should_panic(expected = "outside ±24:00")]
    fn a_synthetic_beyond_24h_offset_zone_panics_at_the_line() {
        // Offset::constant(25) is legal jiff (domain ±25:59) but
        // outside the marker's own domain; marker.rs's
        // broken-invariant posture applies through the seam. No real
        // IANA zone (±14:00 max) can build this clock.
        let clock = MarkerClock::from_instant(
            instant("2026-11-17T13:03:22Z"),
            Offset::constant(25).to_time_zone(),
        );
        let _ = clock.marker_line();
    }

    // ---- day rollover: the date belongs to the zone --------------------------

    #[test]
    fn the_date_rolls_forward_past_midnight_in_a_far_east_zone() {
        // 23:30 UTC is already the next day in Tokyo: the date must
        // come from the zone's calendar, not UTC's — an
        // implementation that printed the UTC date and shifted only
        // the clock would stamp 2026-11-17 here and stay green
        // everywhere else in this suite.
        let clock = MarkerClock::from_instant(
            instant("2026-11-17T23:30:00Z"),
            TimeZone::get("Asia/Tokyo").expect("Asia/Tokyo is in the tzdb"),
        );
        assert_eq!(
            clock.marker_line(),
            "<!-- Created by EunomiaSync 2026-11-18 08:30:00 Asia/Tokyo +09:00 -->"
        );
    }

    #[test]
    fn the_date_rolls_backward_before_midnight_in_a_far_west_zone() {
        // 05:00 UTC is still the previous day in Honolulu (-10:00):
        // the westward rollover — date and negative offset together.
        let clock = MarkerClock::from_instant(
            instant("2026-11-17T05:00:00Z"),
            TimeZone::get("Pacific/Honolulu").expect("Pacific/Honolulu is in the tzdb"),
        );
        assert_eq!(
            clock.marker_line(),
            "<!-- Created by EunomiaSync 2026-11-16 19:00:00 Pacific/Honolulu -10:00 -->"
        );
    }

    // ---- the run entry point's environment guard ------------------------------

    #[test]
    fn a_posix_rule_beyond_the_marker_domain_degrades_to_the_utc_form() {
        // TZ can legally name a POSIX zone whose offset leaves the
        // marker's own ±24:00 domain (jiff parses the rule; real zones
        // stop at ±14:00). The run entry point must not panic on it:
        // the capture degrades to the nameless UTC form at the same
        // instant — wall clock and offset agree.
        let zone = TimeZone::posix("AAA-24:59:59BBB,M3.2.0,M11.1.0")
            .expect("a well-formed POSIX rule with a >24h offset parses");
        let instant = instant("2026-11-17T13:03:22Z"); // the rule's standard time
        let raw = MarkerClock::from_instant(instant, zone.clone());
        assert_eq!(raw.offset_minutes(), 24 * 60 + 59); // outside the domain
        let captured = capture_run_clock(instant, zone);
        assert_eq!(
            captured,
            MarkerClock::from_instant(instant, TimeZone::unknown())
        );
        assert_eq!(
            captured.marker_line(),
            "<!-- Created by EunomiaSync 2026-11-17 13:03:22 UTC+00:00 -->"
        );
    }

    #[test]
    fn a_zone_inside_the_marker_domain_passes_the_run_guard_untouched() {
        // The guard is invisible for every zone a machine can sanely
        // have: the capture is exactly the pure seam's value.
        let instant = instant("2026-11-17T13:03:22Z");
        let zone = TimeZone::get("Europe/Prague").expect("Europe/Prague is in the tzdb");
        assert_eq!(
            capture_run_clock(instant, zone.clone()),
            MarkerClock::from_instant(instant, zone)
        );
    }

    #[test]
    fn the_run_guard_passes_exactly_twenty_four_hours_through() {
        // ±24:00 exactly is inside the marker's domain (marker.rs
        // formats it; only beyond it panics), so the guard must pass
        // it through untouched — if its `<=` ever regressed to `<`, a
        // legal ±24:00 zone would silently degrade to UTC+00:00, and
        // only this boundary pin would notice. The full line checks
        // the pass-through carries the wall clock a whole day over.
        let instant = instant("2026-11-17T13:03:22Z");
        for hours in [24, -24] {
            let zone = Offset::constant(hours).to_time_zone();
            assert_eq!(
                capture_run_clock(instant, zone.clone()),
                MarkerClock::from_instant(instant, zone),
                "the guard must be invisible at exactly ±24:00"
            );
        }
        assert_eq!(
            capture_run_clock(instant, Offset::constant(24).to_time_zone()).marker_line(),
            "<!-- Created by EunomiaSync 2026-11-18 13:03:22 UTC+24:00 -->"
        );
    }

    #[test]
    fn sub_second_precision_does_not_reach_the_marker() {
        // MarkerTime is second-granular; a fractional instant renders
        // truncated, not rounded up into the next second.
        let clock = MarkerClock::from_instant(
            instant("2026-11-17T13:03:22.937Z"),
            TimeZone::get("Europe/Prague").expect("Europe/Prague is in the tzdb"),
        );
        assert_eq!(
            clock.marker_line(),
            "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->"
        );
    }
}
