#!/usr/bin/env node
// License allow-list gate over the installed npm packages — the JavaScript
// twin of cargo-deny's [licenses] check (deny.toml), for the tree pnpm
// installs. Spec §5.3 "License checks", SEC-010; DECISIONS.md D-184.
//
// Input: the JSON of `pnpm licenses list --json` — pnpm's own inventory of
// what `pnpm install --frozen-lockfile` put on disk, shaped
//   { "<license>": [ { name, versions: [...], paths: [...] }, ... ] }
// read from a file, or `-` for stdin. The allow-list is allow-list.txt next
// to this file.
//
// A package passes when the allow-list satisfies its license expression
// under SPDX semantics, read the way cargo-deny reads deny.toml:
//   OR   — one allowed alternative is enough
//   AND  — every operand must be allowed
//   ( )  — group
//   WITH — `X WITH Y` must be listed verbatim
// Everything else fails closed: an id that is not listed (Unknown,
// UNLICENSED, a copyleft or source-available id), a string that is not an
// expression, an unreadable or empty inventory. Nothing is inferred from
// LICENSE files.
//
// Exit codes: 0 — every package is allowed; 1 — at least one is not;
// 2 — a usage error, or input the checker refuses to judge (not JSON, no
// packages, a wrong shape). A gate that cannot see the tree never passes it.
//
// Usage, from the repo root — the same two commands CI's lint job runs,
// with the inventory written outside the repo:
//   pnpm licenses list --json > /tmp/licenses.json
//   node .github/scripts/licenses/check.mjs /tmp/licenses.json
// Tests: node --test .github/scripts/licenses/check.test.mjs
import { readFileSync, realpathSync } from 'node:fs'
import { fileURLToPath, pathToFileURL } from 'node:url'

const ALLOW_LIST_PATH = new URL('./allow-list.txt', import.meta.url)
const ALLOW_LIST_FILE = fileURLToPath(ALLOW_LIST_PATH)
/** The checker cannot judge this input: exit 2, never a silent pass. */
export class InputError extends Error {}

const OPERATORS = new Set(['OR', 'AND', 'WITH'])
// One allow-list line: a license id, optionally `WITH` an exception id. The
// id characters are SPDX's (letters, digits, '.', '-', '+', ':').
const ALLOW_LIST_ENTRY = /^[A-Za-z0-9][A-Za-z0-9.\-+:]*( WITH [A-Za-z0-9][A-Za-z0-9.\-+:]*)?$/
// A real license expression is a few dozen characters. A longer string is
// not one; refusing it outright also keeps the recursive descent shallow.
export const MAX_EXPRESSION_LENGTH = 500

function isPlainObject(value) {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** `#` starts a comment; one entry per line; anything else is refused. */
export function parseAllowList(text) {
  const allow = new Set()
  text.split('\n').forEach((raw, index) => {
    const line = raw.split('#')[0].trim()
    if (line === '') return
    if (!ALLOW_LIST_ENTRY.test(line)) {
      throw new InputError(
        `line ${index + 1}: ${JSON.stringify(line)} is not an SPDX license id (optionally "WITH <exception>")`,
      )
    }
    allow.add(line)
  })
  if (allow.size === 0) throw new InputError('the allow-list is empty')
  return allow
}

// Recursive descent over the tokens, no backtracking: linear in the length
// of the expression. Every branch is parsed even once the verdict is known,
// so a syntactically broken tail is never skipped. A malformed expression
// throws, and the caller reads that as "not allowed".

function parseOr(state, allow) {
  let verdict = parseAnd(state, allow)
  while (state.tokens[state.pos] === 'OR') {
    state.pos += 1
    if (parseAnd(state, allow)) verdict = true
  }
  return verdict
}

function parseAnd(state, allow) {
  let verdict = parseAtom(state, allow)
  while (state.tokens[state.pos] === 'AND') {
    state.pos += 1
    if (!parseAtom(state, allow)) verdict = false
  }
  return verdict
}

function parseAtom(state, allow) {
  const token = state.tokens[state.pos]
  state.pos += 1
  if (token === '(') {
    const verdict = parseOr(state, allow)
    if (state.tokens[state.pos] !== ')') throw new InputError('unclosed group')
    state.pos += 1
    return verdict
  }
  if (token === undefined || token === ')' || OPERATORS.has(token)) {
    throw new InputError(`expected a license id, got ${JSON.stringify(token ?? null)}`)
  }
  let atom = token
  if (state.tokens[state.pos] === 'WITH') {
    state.pos += 1
    const exception = state.tokens[state.pos]
    state.pos += 1
    if (exception === undefined || exception === ')' || OPERATORS.has(exception)) {
      throw new InputError('`WITH` without an exception id')
    }
    atom = `${token} WITH ${exception}`
  }
  return allow.has(atom)
}

/** True iff the allow-list satisfies the license expression. */
export function isAllowed(expression, allow) {
  if (
    typeof expression !== 'string' ||
    expression.length === 0 ||
    expression.length > MAX_EXPRESSION_LENGTH
  ) {
    return false
  }
  const tokens = expression.match(/\(|\)|[^\s()]+/g) ?? []
  const state = { tokens, pos: 0 }
  try {
    const verdict = parseOr(state, allow)
    return verdict && state.pos === tokens.length
  } catch {
    return false
  }
}

/**
 * Judge every package of a `pnpm licenses list --json` inventory.
 * Returns { instances, byLicense, violations }; an inventory the checker
 * cannot read throws InputError instead of passing.
 */
export function checkInventory(inventory, allow) {
  if (!isPlainObject(inventory)) {
    throw new InputError('not a JSON object of license -> packages')
  }
  const licenses = Object.keys(inventory)
  if (licenses.length === 0) throw new InputError('lists no packages')
  const packages = []
  for (const license of licenses) {
    const entries = inventory[license]
    if (!Array.isArray(entries) || entries.length === 0) {
      throw new InputError(`license key ${JSON.stringify(license)} has no packages`)
    }
    for (const entry of entries) {
      if (!isPlainObject(entry) || typeof entry.name !== 'string' || entry.name === '') {
        throw new InputError(
          `license key ${JSON.stringify(license)} has an entry without a package name`,
        )
      }
      if (
        !Array.isArray(entry.versions) ||
        entry.versions.length === 0 ||
        !entry.versions.every((version) => typeof version === 'string')
      ) {
        throw new InputError(`${entry.name}: "versions" is not a non-empty array of strings`)
      }
      packages.push({ name: entry.name, versions: entry.versions, license })
    }
  }
  const byLicense = new Map()
  const violations = []
  let instances = 0
  for (const pkg of packages) {
    instances += pkg.versions.length
    byLicense.set(pkg.license, (byLicense.get(pkg.license) ?? 0) + 1)
    if (!isAllowed(pkg.license, allow)) violations.push(pkg)
  }
  // Code-unit order: the same report on every machine and locale.
  const reportKey = ({ name, versions, license }) => `${name}@${versions.join(',')} ${license}`
  violations.sort((a, b) => {
    const left = reportKey(a)
    const right = reportKey(b)
    return left < right ? -1 : left > right ? 1 : 0
  })
  return { instances, byLicense, violations }
}

function main(argv) {
  const [inventoryPath, ...extra] = argv
  if (inventoryPath === undefined || extra.length > 0) {
    console.error(
      'usage: check.mjs <licenses.json | ->   (the output of `pnpm licenses list --json`)',
    )
    return 2
  }
  let allow
  try {
    allow = parseAllowList(readFileSync(ALLOW_LIST_PATH, 'utf8'))
  } catch (error) {
    console.error(`licenses: allow-list ${ALLOW_LIST_FILE}: ${error.message}`)
    return 2
  }
  let inventory
  try {
    inventory = JSON.parse(readFileSync(inventoryPath === '-' ? 0 : inventoryPath, 'utf8'))
  } catch (error) {
    console.error(`licenses: inventory ${inventoryPath}: ${error.message}`)
    return 2
  }
  let result
  try {
    result = checkInventory(inventory, allow)
  } catch (error) {
    if (!(error instanceof InputError)) throw error
    console.error(`licenses: inventory ${inventoryPath} ${error.message}`)
    return 2
  }
  const summary = [...result.byLicense]
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([license, count]) => `${license} (${count})`)
    .join(', ')
  console.log(
    `licenses: ${result.instances} installed packages under ${result.byLicense.size} license identifiers: ${summary}`,
  )
  if (result.violations.length === 0) {
    console.log('licenses: every package is inside the allow-list')
    return 0
  }
  console.error(
    `licenses: ${result.violations.length} package(s) outside the allow-list (${ALLOW_LIST_FILE}):`,
  )
  for (const pkg of result.violations) {
    console.error(`  ${pkg.name}@${pkg.versions.join(', ')}: ${JSON.stringify(pkg.license)}`)
  }
  console.error('Add the license to the allow-list with the reason for it, or drop the package.')
  return 1
}

const isEntryPoint = (() => {
  try {
    return pathToFileURL(realpathSync(process.argv[1])).href === import.meta.url
  } catch {
    return false
  }
})()

if (isEntryPoint) {
  process.exitCode = main(process.argv.slice(2))
}
