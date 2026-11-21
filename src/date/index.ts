// Date/time formatting helpers (T-144, spec UI-020 / MAIN-027 / PRJ-021).
//
// UI-020: "Dates are formatted with `Intl` in the system locale; time is
// always shown with the time zone." MAIN-027's sync panel and PRJ-021's
// project-settings modal both show the last-sync time as "local time and
// time zone". The zone form chosen here is the UTC offset in the
// `GMT±hh:mm` shape (`timeZoneName: 'longOffset'`) — the spec's own
// offset habit (ALG-032's `UTC+01:00`, DB-002's `Z`), rendered uniformly
// for every zone including UTC (`GMT+00:00`). The IANA-name-plus-offset
// form belongs to the file marker (ALG-032, D-104), not to the UI.
//
// Inputs: `Date`, an epoch-milliseconds number, or a string — the DB-002
// wire form (`2026-11-17T13:03:22.123Z`, UTC ISO-8601 with milliseconds)
// parses through `new Date`. Milliseconds are not rendered (medium
// precision stops at seconds); they are preserved in the instant.
//
// Locale is a PARAMETER, never a global: `undefined` (the default) is
// Intl's own system-locale default. The zone is a parameter too — an
// IANA name (`Europe/Prague`) or a fixed-offset identifier in the
// `±hh:mm` form (`+05:45`; the `UTC±h` spelling is outside the engine's
// accepted subset and fails validation like any unknown name). When no
// zone is given, the option is omitted so Intl uses the system zone
// natively — including its graceful degradation for unidentifiable
// system zones (`Etc/Unknown` renders as UTC).
//
// Errors are honest and typed: an unparseable date or an unusable zone
// throws `DateFormatError` with a `kind` discriminant, so callers branch
// on `error.kind`, not on message text. The engine's own `RangeError`
// rides along as `cause`. A structurally invalid locale is NOT wrapped:
// the locale passes straight through to Intl and its errors stay the
// engine's (our validation probe uses a fixed valid locale, so a locale
// failure can never be misattributed as a zone failure).
//
// Implementation notes:
// - The date-time form is built from explicit components, not
//   `dateStyle`/`timeStyle` — ECMA-402 throws a TypeError when either
//   style option is combined with any component option, and
//   `timeZoneName` (the UI-020 "always with the time zone" half) is one.
// - `timeZoneName: 'longOffset'` needs WebKit ≥ 15.4 (the Tauri
//   WKWebView on macOS ≥ 12.3; NFR-001's floor is macOS 12). Older
//   engines throw a RangeError on the option value at construction, so
//   it is feature-detected once at module load and degrades to `'short'`
//   (`GMT+1`, `CEST`): still an explicit zone marker per UI-020 — only
//   the exact `±hh:mm` shape is lost on those WebKits.
// - Formatters are cached per (shape, locale, zone): construction costs
//   ~100× a cached `format()` call (measured ~0.36 ms vs ~3.6 µs on
//   Node 22), and the matrix view may format hundreds of timestamps per
//   render. The cache is capped and cleared wholesale — realistic key
//   sets are tiny (the system locale plus a handful of zones), and the
//   overflow path only trades speed, never output.
//
// Determinism: every behavior is a pure function of (instant, locale,
// zone); nothing reads the process environment except the documented
// `undefined` defaults (system locale, system zone), `systemTimeZone`,
// and the one-time `longOffset` probe.

/** What the helpers accept: a `Date`, a DB-002 ISO-8601 string, or epoch milliseconds. */
export type DateInput = Date | string | number

/** Locale selection passed straight to `Intl` (`undefined` = system locale). */
export type LocaleInput = Intl.LocalesArgument

/** Options shared by both formatters. */
export interface FormatDateOptions {
  /**
   * Locale(s) for `Intl.DateTimeFormat`. `undefined` — the default — is
   * Intl's own system-locale default (UI-020: "the system locale").
   */
  locale?: LocaleInput
  /**
   * Zone the calendar is rendered in: an IANA name (`Europe/Prague`) or
   * a fixed-offset identifier (`+05:45`). `undefined` — the default —
   * lets Intl use the system zone (MAIN-027/PRJ-021: local time).
   */
  timeZone?: string
}

/** Which input was rejected (callers branch on `kind`, not on messages). */
export type DateFormatErrorKind = 'invalid-date' | 'invalid-time-zone'

/** Thrown for an unparseable date input or an unusable time-zone identifier. */
export class DateFormatError extends Error {
  readonly kind: DateFormatErrorKind
  /** The engine's original `RangeError`, kept for diagnostics. */
  readonly cause: unknown

  constructor(kind: DateFormatErrorKind, message: string, cause?: unknown) {
    super(message)
    this.name = 'DateFormatError'
    this.kind = kind
    this.cause = cause
  }
}

// The `longOffset` zone-name style with the pre-15.4-WebKit fallback
// (see the module notes); computed once.
const ZONE_NAME_STYLE: NonNullable<Intl.DateTimeFormatOptions['timeZoneName']> = (() => {
  try {
    new Intl.DateTimeFormat('en', { timeZoneName: 'longOffset' })
    return 'longOffset'
  } catch {
    return 'short'
  }
})()

// The UI-020 "always with the time zone" form: explicit components (see
// the module notes about dateStyle/timeStyle) plus the zone name.
const DATE_TIME_COMPONENTS: Intl.DateTimeFormatOptions = {
  year: 'numeric',
  month: 'short',
  day: 'numeric',
  hour: 'numeric',
  minute: '2-digit',
  second: '2-digit',
  timeZoneName: ZONE_NAME_STYLE,
}

/**
 * Formats a calendar date in the zone's own calendar — no time, no zone
 * name (a date carries no wall time to qualify). The optional `timeZone`
 * still decides WHICH day is shown: `2026-11-17T23:30:00Z` is Nov 17 in
 * New York and Nov 18 in Tokyo.
 */
export function formatDate(date: DateInput, options: FormatDateOptions = {}): string {
  const instant = toDate(date)
  const { locale, timeZone } = options
  if (timeZone !== undefined) assertUsableTimeZone(timeZone)
  // `timeZone` is not a date-time component, so it composes with dateStyle.
  return formatterFor('date', locale, timeZone).format(instant)
}

/**
 * Formats a date-time as local wall time with the zone offset — the
 * offset in force at that instant in that zone (DST included), always
 * rendered (UI-020/MAIN-027/PRJ-021), e.g. `Nov 17, 2026, 2:03:22 PM
 * GMT+01:00`.
 */
export function formatDateTime(date: DateInput, options: FormatDateOptions = {}): string {
  const instant = toDate(date)
  const { locale, timeZone } = options
  if (timeZone !== undefined) assertUsableTimeZone(timeZone)
  return formatterFor('date-time', locale, timeZone).format(instant)
}

/**
 * The system zone as a usable identifier: what
 * `Intl.DateTimeFormat().resolvedOptions().timeZone` resolved to, or a
 * synthesized fixed-offset identifier when the zone cannot be named —
 * a fixed-offset POSIX `TZ` (e.g. `TZ=UTC-3` applies +03:00 but resolves
 * to no name), or an unusable name like `Etc/Unknown`, which the engine
 * rejects as an explicit option while happily defaulting to it. POSIX
 * rules with transitions (e.g. `PST8PDT,…`) resolve to real IANA names
 * in ICU, so the synthesized arm is exact wherever it fires; the offset
 * is read at the call moment regardless (documented for completeness).
 * The return value is always safe to pass back in
 * `FormatDateOptions.timeZone`.
 */
export function systemTimeZone(): string {
  const resolved = Intl.DateTimeFormat().resolvedOptions().timeZone
  if (resolved !== undefined) {
    try {
      assertUsableTimeZone(resolved)
      return resolved
    } catch {
      // Unusable name — synthesize from the zone's actual offset below.
    }
  }
  return offsetTimeZoneIdentifier(-new Date().getTimezoneOffset())
}

/**
 * Builds the `±hh:mm` fixed-offset identifier (the `timeZone` spelling
 * Intl accepts) from an offset in minutes east of UTC — the domain of
 * `-Date.prototype.getTimezoneOffset()`.
 */
export function offsetTimeZoneIdentifier(offsetMinutes: number): string {
  const sign = offsetMinutes < 0 ? '-' : '+'
  const abs = Math.abs(offsetMinutes)
  const hours = String(Math.floor(abs / 60)).padStart(2, '0')
  const minutes = String(abs % 60).padStart(2, '0')
  return `${sign}${hours}:${minutes}`
}

/** Parses the input shape and rejects unparseable values with a typed error. */
function toDate(date: DateInput): Date {
  const instant = date instanceof Date ? date : new Date(date)
  if (Number.isNaN(instant.getTime())) {
    throw new DateFormatError('invalid-date', `Invalid date value for formatting: ${String(date)}`)
  }
  return instant
}

/**
 * Rejects zone identifiers the engine cannot use, with the typed error.
 * A fixed valid locale keeps the probe honest: any throw here is the
 * zone's, never a locale's.
 */
function assertUsableTimeZone(timeZone: string): void {
  try {
    new Intl.DateTimeFormat('en', { timeZone })
  } catch (cause) {
    throw new DateFormatError(
      'invalid-time-zone',
      `Invalid time zone identifier: "${timeZone}" (an IANA name or a ±hh:mm offset)`,
      cause,
    )
  }
}

/** Formatter cache: see the module notes (construction ≈ 100× format()). */
const FORMATTER_CACHE = new Map<string, Intl.DateTimeFormat>()
const FORMATTER_CACHE_CAP = 64

function formatterFor(
  shape: 'date' | 'date-time',
  locale: LocaleInput | undefined,
  timeZone: string | undefined,
): Intl.DateTimeFormat {
  const key = `${shape}|${localeCacheKey(locale)}|${timeZone ?? ''}`
  let formatter = FORMATTER_CACHE.get(key)
  if (formatter === undefined) {
    formatter =
      shape === 'date'
        ? // `timeZone` is not a component option, so it composes with
          // `dateStyle` (unlike with the date-time component set).
          new Intl.DateTimeFormat(locale, { dateStyle: 'medium', timeZone })
        : new Intl.DateTimeFormat(locale, { ...DATE_TIME_COMPONENTS, timeZone })
    if (FORMATTER_CACHE.size >= FORMATTER_CACHE_CAP) FORMATTER_CACHE.clear()
    FORMATTER_CACHE.set(key, formatter)
  }
  return formatter
}

function localeCacheKey(locale: LocaleInput | undefined): string {
  if (locale === undefined) return ''
  // JSON of pre-stringified parts is collision-free across the
  // LocalesArgument shapes: a string key is quoted (`"en,fr"`), an
  // array key is bracketed (`["en","fr"]`), and `Intl.Locale` objects
  // collapse to their language tag (round-2 review: a plain join let
  // the string `'en,fr'` silently reuse the array's formatter).
  return JSON.stringify(Array.isArray(locale) ? locale.map(String) : String(locale))
}
