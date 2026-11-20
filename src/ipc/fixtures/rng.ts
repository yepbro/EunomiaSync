// Deterministic randomness for IPC fixtures (T-141).
//
// The generator must be reproducible: the same profile (seed included)
// yields structurally identical data on every run, on every machine — that
// is what makes a failing scroll test against set L repeatable. `Math.random`
// and `Date.now` are therefore banned from fixture code; everything flows
// from the seeded PRNG and a fixed base timestamp.
//
// mulberry32 (public domain): 32-bit state, one draw per call — enough
// randomness for fixtures at negligible cost, with no dependency
// (new dependencies are an owner decision; T-141 stays on the existing
// stack).

/** A seeded pseudo-random source. Deterministic per seed. */
export type Rng = {
  /** Next float in [0, 1). */
  next(): number
  /** Uniform integer in [min, max], both inclusive. */
  int(min: number, max: number): number
  /** Uniform `true`/`false`. */
  bool(): boolean
  /** A uniform member of a non-empty list. */
  pick<T>(items: readonly T[]): T
}

function mulberry32(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a |= 0
    a = (a + 0x6d2b79f5) | 0
    let t = Math.imul(a ^ (a >>> 15), 1 | a)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

/** Creates a deterministic RNG. The seed is normalized to uint32. */
export function createRng(seed: number): Rng {
  const next = mulberry32(seed)
  return {
    next,
    int(min, max) {
      if (!Number.isInteger(min) || !Number.isInteger(max) || min > max) {
        throw new Error(`invalid int() bounds: ${min}..${max}`)
      }
      return min + Math.floor(next() * (max - min + 1))
    },
    bool() {
      return next() < 0.5
    },
    pick<T>(items: readonly T[]): T {
      if (items.length === 0) {
        throw new Error('pick() of an empty list')
      }
      return items[Math.floor(next() * items.length)] as T
    },
  }
}

/**
 * A UUIDv7-shaped id (DB-002: ids are "UUIDv7 TEXT"): millisecond timestamp
 * in the high 48 bits, version nibble `7`, variant nibble `8..b`, the rest
 * random. Deterministic given the same rng draws and timestamp — no
 * `crypto`, no wall clock.
 */
export function uuidv7From(rng: Rng, tsMs: number): string {
  const safeTs = Math.max(0, Math.floor(tsMs))
  // 48-bit timestamp as 12 hex digits, without BigInt (lib target ES2020).
  const tsHi = Math.floor(safeTs / 2 ** 32)
  const tsLo = safeTs % 2 ** 32
  const ts = (tsHi.toString(16).padStart(4, '0') + tsLo.toString(16).padStart(8, '0')).slice(-12)
  const word = () => rng.int(0, 0xffff).toString(16).padStart(4, '0')
  const rand = word() + word() + word() + word() + word() // 20 hex digits
  // Overwrite the version nibble (index 12) and the variant nibble (16) in
  // the flat 32-digit string, then place the dashes 8-4-4-4-12.
  const hex = ts + '7' + rand.slice(1, 4) + rng.int(8, 11).toString(16) + rand.slice(5)
  return [
    hex.slice(0, 8),
    hex.slice(8, 12),
    hex.slice(12, 16),
    hex.slice(16, 20),
    hex.slice(20, 32),
  ].join('-')
}

/** ISO-8601 UTC (`YYYY-MM-DDTHH:MM:SS.sssZ`, the DB-002 wire shape). */
export function isoUtc(tsMs: number): string {
  return new Date(tsMs).toISOString()
}
