import { describe, expect, it } from 'vitest'
import {
  DateFormatError,
  formatDate,
  formatDateTime,
  offsetTimeZoneIdentifier,
  systemTimeZone,
} from './index'

// Determinism rules for this suite: every instant is a fixed UTC string
// (the DB-002 wire form), every zone is explicit, every locale is
// explicit — no expectation depends on the runner's TZ or language.
// Prague/New York carry DST transitions around the picked instants;
// Tokyo and Accra had no DST in 2026 (fixed offsets); `+05:45` is a
// fixed-offset identifier (Nepal's offset, without naming a zone).
// The only system-derived checks are the default-equivalence pins,
// which hold on any runner by construction (both sides read the same
// system default — verified for named zones, `TZ=` → Etc/Unknown,
// `TZ=Invalid/Zone` → no resolved name, and the fixed-offset POSIX
// `TZ=UTC-3`, whose synthesized `+03:00` identifier matches the
// engine's default rendering). Expected strings were verified against
// Node 22's ICU (V8 renders `longOffset` uniformly, UTC included).

// The DB-002 example instant, milliseconds included (not rendered).
const T = '2026-11-17T13:03:22.123Z'

describe('formatDateTime', () => {
  it('renders the zone wall time with its offset for an IANA zone', () => {
    // 13:03:22Z is 14:03:22 in Prague (CET, +01:00); the milliseconds of
    // the wire string stay in the instant but are not rendered.
    expect(formatDateTime(T, { locale: 'en', timeZone: 'Europe/Prague' })).toBe(
      'Nov 17, 2026, 2:03:22 PM GMT+01:00',
    )
  })

  it('renders a negative-offset zone', () => {
    expect(formatDateTime(T, { locale: 'en', timeZone: 'America/New_York' })).toBe(
      'Nov 17, 2026, 8:03:22 AM GMT-05:00',
    )
  })

  it('renders a zone without DST at its fixed offset', () => {
    expect(formatDateTime(T, { locale: 'en', timeZone: 'Asia/Tokyo' })).toBe(
      'Nov 17, 2026, 10:03:22 PM GMT+09:00',
    )
    expect(formatDateTime(T, { locale: 'en', timeZone: 'Africa/Accra' })).toBe(
      'Nov 17, 2026, 1:03:22 PM GMT+00:00',
    )
  })

  it('renders UTC itself as an explicit zero offset', () => {
    expect(formatDateTime(T, { locale: 'en', timeZone: 'UTC' })).toBe(
      'Nov 17, 2026, 1:03:22 PM GMT+00:00',
    )
  })

  it('accepts a fixed-offset zone identifier', () => {
    expect(formatDateTime(T, { locale: 'en', timeZone: '+05:45' })).toBe(
      'Nov 17, 2026, 6:48:22 PM GMT+05:45',
    )
  })

  it('uses the DST offset in force at the instant (fall-back: same wall clock, offset flips)', () => {
    // Europe/Prague left CEST on 2026-10-25 at 03:00 → 02:00: both
    // instants below show 2:30 AM with a different offset.
    expect(
      formatDateTime('2026-10-25T00:30:00Z', { locale: 'en', timeZone: 'Europe/Prague' }),
    ).toBe('Oct 25, 2026, 2:30:00 AM GMT+02:00')
    expect(
      formatDateTime('2026-10-25T01:30:00Z', { locale: 'en', timeZone: 'Europe/Prague' }),
    ).toBe('Oct 25, 2026, 2:30:00 AM GMT+01:00')
  })

  it('uses the DST offset in force at the instant (spring-forward: the wall clock jumps)', () => {
    // America/New_York entered EDT on 2026-03-08 at 02:00 → 03:00.
    expect(
      formatDateTime('2026-03-08T06:30:00Z', { locale: 'en', timeZone: 'America/New_York' }),
    ).toBe('Mar 8, 2026, 1:30:00 AM GMT-05:00')
    expect(
      formatDateTime('2026-03-08T07:30:00Z', { locale: 'en', timeZone: 'America/New_York' }),
    ).toBe('Mar 8, 2026, 3:30:00 AM GMT-04:00')
  })

  it('always shows the zone: every zone form ends with a GMT±hh:mm offset (UI-020)', () => {
    // The zone marker is part of the output for every usable zone
    // spelling — dropping `timeZoneName` fails this, not just the exact
    // string pins above.
    const zones = [
      'Europe/Prague',
      'America/New_York',
      'Asia/Tokyo',
      'Africa/Accra',
      'UTC',
      '+05:45',
    ]
    for (const timeZone of zones) {
      expect(formatDateTime(T, { locale: 'en', timeZone })).toMatch(/GMT[+-]\d{2}:\d{2}$/)
    }
  })

  it('formats per the explicit locale', () => {
    // Exact pins only on platform-stable facts (D-111 spirit): the
    // 24-hour wall clock (en renders 2:03:22 PM, cs-CZ 14:03:22) and
    // the GMT±hh:mm suffix are the spec-relevant halves; month
    // abbreviations drift with ICU versions, so they are not pinned.
    // The all-numeric cs-CZ date (in the formatDate suite) is stable
    // and pinned exactly.
    const pragueCz = { locale: 'cs-CZ' as const, timeZone: 'Europe/Prague' }
    expect(formatDateTime(T, pragueCz)).toContain('14:03:22')
    expect(formatDateTime(T, pragueCz)).toContain('GMT+01:00')
    expect(formatDateTime(T, { locale: 'de-DE', timeZone: 'Europe/Prague' })).toContain('14:03:22')
  })

  it('accepts Date, number, and string inputs for the same instant', () => {
    const expected = 'Nov 17, 2026, 2:03:22 PM GMT+01:00'
    const opts = { locale: 'en' as const, timeZone: 'Europe/Prague' }
    expect(formatDateTime(new Date(T), opts)).toBe(expected)
    expect(formatDateTime(new Date(T).getTime(), opts)).toBe(expected)
    expect(formatDateTime(T, opts)).toBe(expected)
  })

  it('defaults to the system zone when no zone is given (MAIN-027/PRJ-021)', () => {
    // Runner-independent pin: the default and the explicit system zone
    // must render identically on every machine, whatever its TZ is
    // (both sides read the same system default; an unidentified system
    // zone degrades to UTC on both sides alike).
    expect(formatDateTime(T, { locale: 'en' })).toBe(
      formatDateTime(T, { locale: 'en', timeZone: systemTimeZone() }),
    )
  })

  it('defaults to the system locale when no locale is given (UI-020)', () => {
    // Same equivalence shape as the zone default: the parameter-less
    // call must be the system-locale call on any runner.
    const systemLocale = Intl.DateTimeFormat().resolvedOptions().locale
    expect(formatDateTime(T)).toBe(formatDateTime(T, { locale: systemLocale }))
  })
})

describe('formatDate', () => {
  it('renders a calendar date without a time or a zone name', () => {
    expect(formatDate(T, { locale: 'en', timeZone: 'Europe/Prague' })).toBe('Nov 17, 2026')
  })

  it('rolls the calendar day to the zone', () => {
    // 23:30Z is still Nov 17 in New York but already Nov 18 in Tokyo —
    // a date-only rendering still needs the zone to pick the day.
    expect(formatDate('2026-11-17T23:30:00Z', { locale: 'en', timeZone: 'America/New_York' })).toBe(
      'Nov 17, 2026',
    )
    expect(formatDate('2026-11-17T23:30:00Z', { locale: 'en', timeZone: 'Asia/Tokyo' })).toBe(
      'Nov 18, 2026',
    )
  })

  it('formats per the explicit locale', () => {
    // All-numeric and therefore stable across ICU versions, unlike
    // month abbreviations (see the dateTime locale test).
    expect(formatDate(T, { locale: 'cs-CZ', timeZone: 'Europe/Prague' })).toBe('17. 11. 2026')
  })

  it('defaults to the system zone when no zone is given', () => {
    // Same runner-independent equivalence as the date-time default.
    expect(formatDate(T)).toBe(formatDate(T, { timeZone: systemTimeZone() }))
  })
})

describe('systemTimeZone', () => {
  it('returns a zone identifier that is safe to pass back in', () => {
    const zone = systemTimeZone()
    expect(typeof zone).toBe('string')
    expect(zone.length).toBeGreaterThan(0)
    // The contract of the return value: usable as an explicit option.
    expect(() => formatDateTime(T, { locale: 'en', timeZone: zone })).not.toThrow()
  })
})

describe('the error contract (DateFormatError)', () => {
  // Narrows the throw to the typed error without casts; fails the test
  // outright when the call returns or throws something else.
  function catchAsDateFormatError(fn: () => unknown): DateFormatError {
    try {
      fn()
    } catch (caught) {
      if (caught instanceof DateFormatError) return caught
      throw new Error(`not a DateFormatError: ${String(caught)}`)
    }
    throw new Error('expected a throw, got a return')
  }

  it('rejects an unparseable date with a typed error naming the input', () => {
    expect(() => formatDate('yesterday')).toThrow(DateFormatError)
    const error = catchAsDateFormatError(() => formatDateTime('yesterday'))
    expect(error.kind).toBe('invalid-date')
    expect(error.message).toContain('yesterday')
    // An already-invalid Date and a NaN epoch fail the same way.
    expect(() => formatDate(new Date(NaN))).toThrow(DateFormatError)
    expect(() => formatDateTime(Number.NaN)).toThrow(DateFormatError)
  })

  it('rejects an unusable zone with a typed error naming the zone', () => {
    for (const timeZone of ['Not/AZone', '', 'UTC+2', 'Etc/Unknown']) {
      const error = catchAsDateFormatError(() => formatDateTime(T, { locale: 'en', timeZone }))
      expect(error.kind).toBe('invalid-time-zone')
      expect(error.message).toContain(timeZone === '' ? '""' : timeZone)
      expect(error.cause).toBeInstanceOf(RangeError)
    }
  })

  it('validates the zone for date-only formatting too', () => {
    expect(() => formatDate(T, { locale: 'en', timeZone: 'Not/AZone' })).toThrow(DateFormatError)
  })

  it('lets the engine speak for a structurally invalid locale (not wrapped)', () => {
    // A zone is present, so the fixed-'en' validation probe runs first
    // and passes — the caller's invalid locale then surfaces at real
    // formatter construction as the engine's own RangeError, never
    // misattributed to the zone (DateFormatError extends Error, not
    // RangeError, so this also proves it is not our typed error).
    expect(() => formatDateTime(T, { locale: 'en_US', timeZone: 'Europe/Prague' })).toThrow(
      RangeError,
    )
  })
})

describe('offsetTimeZoneIdentifier', () => {
  it('builds the ±hh:mm identifier Intl accepts, for the getTimezoneOffset domain', () => {
    // Minutes EAST of UTC = the negated Date#getTimezoneOffset() value;
    // +03:00 is exactly what a `TZ=UTC-3` runner synthesizes.
    expect(offsetTimeZoneIdentifier(180)).toBe('+03:00')
    expect(offsetTimeZoneIdentifier(-210)).toBe('-03:30')
    expect(offsetTimeZoneIdentifier(345)).toBe('+05:45')
    expect(offsetTimeZoneIdentifier(0)).toBe('+00:00')
  })

  it('produces identifiers the formatter accepts', () => {
    for (const identifier of ['+03:00', '-03:30', '+05:45', '+00:00']) {
      expect(formatDateTime(T, { locale: 'en', timeZone: identifier })).toMatch(
        /GMT[+-]\d{2}:\d{2}$/,
      )
    }
  })
})
