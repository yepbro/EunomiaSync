// Unit tests for the license allow-list checker. Zero dependencies on
// purpose — the checker has none either: `node --test` runs them, in CI
// (lint job) and locally (DECISIONS.md D-184).
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import {
  InputError,
  MAX_EXPRESSION_LENGTH,
  checkInventory,
  isAllowed,
  parseAllowList,
} from './check.mjs'

const script = fileURLToPath(new URL('./check.mjs', import.meta.url))
const realAllowList = readFileSync(new URL('./allow-list.txt', import.meta.url), 'utf8')
// deny.toml is the independent oracle for the Rust half of the list.
const denyToml = readFileSync(new URL('../../../deny.toml', import.meta.url), 'utf8')

function pkg(name, versions = ['1.0.0']) {
  return { name, versions, paths: [] }
}

function run(args, input) {
  const result = spawnSync(process.execPath, [script, ...args], { encoding: 'utf8', input })
  assert.equal(result.error, undefined, `the checker did not run: ${result.error}`)
  return { status: result.status, stdout: result.stdout, stderr: result.stderr }
}

test('parseAllowList: comments, blank lines and entries; a malformed line is refused', () => {
  const allow = parseAllowList(
    '# a comment\n\n  MIT  \nApache-2.0 # a trailing comment\nApache-2.0 WITH LLVM-exception\n',
  )
  assert.deepEqual([...allow], ['MIT', 'Apache-2.0', 'Apache-2.0 WITH LLVM-exception'])
  // An expression is not an entry, and neither is prose, another tool's
  // list syntax (license-checker's semicolons) or an empty list.
  for (const bad of [
    'MIT OR Apache-2.0',
    'MIT License',
    'MIT;Apache-2.0',
    'MIT, Apache-2.0',
    '# nothing but a comment\n',
    '',
  ]) {
    assert.throws(() => parseAllowList(bad), InputError, JSON.stringify(bad))
  }
})

test('isAllowed: the SPDX reading cargo-deny gives deny.toml', () => {
  const allow = new Set(['MIT', 'Apache-2.0', 'Apache-2.0 WITH LLVM-exception', 'BSD-2-Clause'])
  const allowed = [
    'MIT',
    'Apache-2.0 OR MIT', // one listed alternative is enough
    'GPL-3.0-only OR MIT', // ...even when the other one never will be
    '(MIT OR Apache-2.0) AND BSD-2-Clause',
    '(MIT OR GPL-3.0-only) AND BSD-2-Clause',
    'Apache-2.0 WITH LLVM-exception', // listed verbatim
    'Apache-2.0  OR   MIT', // whitespace is not syntax
    // AND binds tighter than OR (the SPDX grammar):
    // (Apache-2.0 WITH LLVM-exception) OR (GPL-3.0-only AND ISC).
    'Apache-2.0 WITH LLVM-exception OR GPL-3.0-only AND ISC',
    'GPL-3.0-only OR BSD-2-Clause AND MIT',
  ]
  for (const expression of allowed) {
    assert.equal(isAllowed(expression, allow), true, expression)
  }
  const refused = [
    'GPL-3.0-only',
    'MIT AND GPL-3.0-only', // every operand must be listed
    '(MIT OR Apache-2.0) AND GPL-3.0-only',
    'Apache-2.0 WITH Bison-exception-2.2', // an exception is listed or it fails
    'GPL-3.0-only OR ISC AND MIT', // GPL-3.0-only OR (ISC AND MIT): both false
    // Not an expression, or broken: fail closed.
    'Unknown',
    'UNLICENSED',
    'SEE LICENSE IN LICENSE',
    'MIT License',
    'mit',
    '',
    'MIT OR',
    'OR MIT',
    '(MIT',
    'MIT)',
    '()',
    'MIT AND',
    'MIT WITH',
    'WITH MIT',
    'MIT OR OR MIT',
    'MIT+Apache-2.0', // the `+` synonym of AND is not parsed
    'MIT, Apache-2.0',
    null,
    undefined,
    42,
  ]
  for (const expression of refused) {
    assert.equal(isAllowed(expression, allow), false, String(expression))
  }
})

test('isAllowed: long and hostile expressions are judged in bounded work', () => {
  const allow = new Set(['MIT'])
  assert.equal(MAX_EXPRESSION_LENGTH, 500)
  // A license expression is a few dozen characters; one made of nothing but
  // listed ids is still refused once it is past the cap (the cap itself is
  // inside).
  const orChain = (count) => Array(count).fill('MIT').join(' OR ')
  const atTheCap = orChain(72)
  const overTheCap = orChain(73)
  assert.equal(atTheCap.length, MAX_EXPRESSION_LENGTH)
  assert.equal(overTheCap.length, MAX_EXPRESSION_LENGTH + 7)
  assert.equal(isAllowed(atTheCap, allow), true)
  assert.equal(isAllowed(overTheCap, allow), false)
  // Deep nesting under the cap is parsed; past it, nothing is parsed at all.
  assert.equal(isAllowed(`${'('.repeat(200)}MIT${')'.repeat(200)}`, allow), true)
  assert.equal(isAllowed(`${'('.repeat(200_000)}MIT${')'.repeat(200_000)}`, allow), false)
})

test('checkInventory: passes the pnpm licenses list shape and counts by license', () => {
  const result = checkInventory(
    {
      MIT: [pkg('a'), { ...pkg('b', ['2.0.0', '2.1.0']), license: 'MIT', author: 'someone' }],
      'Apache-2.0 OR MIT': [pkg('c')],
    },
    new Set(['MIT']),
  )
  assert.equal(result.instances, 4)
  assert.deepEqual(
    [...result.byLicense],
    [
      ['MIT', 2],
      ['Apache-2.0 OR MIT', 1],
    ],
  )
  assert.deepEqual(result.violations, [])
})

test('checkInventory: a package outside the list is a violation that names it', () => {
  const result = checkInventory(
    {
      MIT: [pkg('fine')],
      'GPL-3.0-only': [pkg('copyleft-kit', ['1.0.0', '1.1.0'])],
      Unknown: [pkg('no-license-field')],
      '': [pkg('empty-license')],
    },
    new Set(['MIT']),
  )
  assert.deepEqual(
    result.violations.map(({ name, versions, license }) => ({ name, versions, license })),
    [
      { name: 'copyleft-kit', versions: ['1.0.0', '1.1.0'], license: 'GPL-3.0-only' },
      { name: 'empty-license', versions: ['1.0.0'], license: '' },
      { name: 'no-license-field', versions: ['1.0.0'], license: 'Unknown' },
    ],
  )
})

test('checkInventory: refuses to judge an inventory it cannot read', () => {
  const allow = new Set(['MIT'])
  for (const inventory of [
    {},
    { MIT: [] },
    { MIT: [pkg('a')], '': [] },
    { MIT: [{ versions: ['1.0.0'] }] },
    { MIT: [{ name: '', versions: ['1.0.0'] }] },
    { MIT: [{ name: 'a', versions: [] }] },
    { MIT: [{ name: 'a', versions: [1] }] },
    { MIT: 'a' },
  ]) {
    assert.throws(() => checkInventory(inventory, allow), InputError, JSON.stringify(inventory))
  }
  for (const inventory of [null, undefined, [], 'MIT', 42]) {
    assert.throws(() => checkInventory(inventory, allow), InputError, String(inventory))
  }
})

test('the real allow-list: deny.toml licenses plus exactly the two npm-only additions', () => {
  const allow = parseAllowList(realAllowList)
  const licensesSection = denyToml.match(/\[licenses\]([\s\S]*?)(?=\n\[|$)/)
  assert.notEqual(licensesSection, null, 'deny.toml has no [licenses] section')
  const denyAllows = [...licensesSection[1].matchAll(/"([^"]+)"/g)].map((match) => match[1])
  assert.ok(denyAllows.length > 0, 'deny.toml [licenses] allow is empty')
  for (const license of denyAllows) {
    assert.ok(allow.has(license), `deny.toml allows ${license}, allow-list.txt does not`)
  }
  const additions = [...allow].filter((license) => !denyAllows.includes(license))
  assert.deepEqual(additions, ['BlueOak-1.0.0', 'Python-2.0'])
})

test('CLI: exit codes — 0 allowed, 1 a violation, 2 a usage or input error', () => {
  const dir = mkdtempSync(join(tmpdir(), 'licenses-check-'))
  try {
    const write = (name, content) => {
      const path = join(dir, name)
      writeFileSync(path, content)
      return path
    }
    const good = JSON.stringify({ MIT: [pkg('a')], ISC: [pkg('b')] })
    const bad = JSON.stringify({ MIT: [pkg('a')], 'GPL-3.0-only': [pkg('copyleft-kit')] })
    const goodPath = write('good.json', good)
    const badPath = write('bad.json', bad)
    const notInstalled = write(
      'not-installed.json',
      JSON.stringify({ Unknown: [pkg('no-license-field')] }),
    )
    const notJson = write('not-json.txt', 'this is not json')
    const empty = write('empty.json', '{}')

    for (const [label, result] of [
      ['file', run([goodPath])],
      ['stdin', run(['-'], good)],
    ]) {
      assert.equal(result.status, 0, `${label}: ${result.stderr}`)
      assert.match(
        result.stdout,
        /2 installed packages under 2 license identifiers: ISC \(1\), MIT \(1\)/,
      )
      assert.match(result.stdout, /every package is inside the allow-list/)
    }

    for (const [label, result] of [
      ['file', run([badPath])],
      ['stdin', run(['-'], bad)],
    ]) {
      assert.equal(result.status, 1, `${label}: ${result.stdout}`)
      assert.match(result.stderr, /copyleft-kit@1\.0\.0: "GPL-3\.0-only"/)
      assert.doesNotMatch(result.stdout, /every package is inside/)
    }

    const unknown = run([notInstalled])
    assert.equal(unknown.status, 1, unknown.stdout)
    assert.match(unknown.stderr, /no-license-field@1\.0\.0: "Unknown"/)

    const garbage = run([notJson])
    assert.equal(garbage.status, 2, garbage.stdout)
    const noPackages = run([empty])
    assert.equal(noPackages.status, 2, noPackages.stdout)
    assert.match(noPackages.stderr, /lists no packages/)
    const noArgument = run([])
    assert.equal(noArgument.status, 2, noArgument.stdout)
    assert.match(noArgument.stderr, /usage: check\.mjs/)
    const tooMany = run([goodPath, badPath])
    assert.equal(tooMany.status, 2, tooMany.stdout)
    const missingFile = run([join(dir, 'does-not-exist.json')])
    assert.equal(missingFile.status, 2, missingFile.stdout)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})
