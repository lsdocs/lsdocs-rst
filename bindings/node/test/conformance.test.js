'use strict'

// The conformance cases of the Rust crate, through the Node package: each
// case parses to docutils' trees (crates/rst/tests/cases, written by
// docutils with `nur gen rst`).

const test = require('node:test')
const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')

const rst = require('..')

const CASES = path.resolve(__dirname, '../../../crates/rst/tests/cases')

/** @param {string} name */
function pending(name) {
  const file = path.join(CASES, name)
  if (!fs.existsSync(file)) return new Set()
  return new Set(
    fs
      .readFileSync(file, 'utf8')
      .split('\n')
      .map((l) => l.trim())
      .filter((l) => l && !l.startsWith('#')),
  )
}

/** @param {string} stem @param {boolean} transforms */
function parser(stem, transforms) {
  const settingsFile = path.join(CASES, `${stem}.settings.json`)
  const settings = fs.existsSync(settingsFile) ? JSON.parse(fs.readFileSync(settingsFile, 'utf8')) : undefined
  const root = stem.startsWith('file_') ? CASES : undefined
  return new rst.Parser({ source: 'test data', transforms, settings, root })
}

/** @param {string} stem @param {string} ext */
function expected(stem, ext) {
  return fs.readFileSync(path.join(CASES, `${stem}.${ext}`), 'utf8')
}

const stems = fs
  .readdirSync(CASES)
  .filter((f) => f.endsWith('.rst'))
  .map((f) => f.slice(0, -4))
  .sort()

test('there are cases', () => {
  assert.ok(stems.length > 50)
})

const parserPending = pending('PENDING')
const transformsPending = pending('PENDING-TRANSFORMS')

for (const stem of stems) {
  const text = fs.readFileSync(path.join(CASES, `${stem}.rst`), 'utf8')
  test(`parser: ${stem}`, { skip: parserPending.has(stem) && 'pending in the Rust crate' }, () => {
    assert.equal(rst.pformat(parser(stem, false).parse(text)), expected(stem, 'pxml'))
  })
  test(`transforms and XML: ${stem}`, { skip: transformsPending.has(stem) && 'pending in the Rust crate' }, () => {
    const doc = parser(stem, true).parse(text)
    assert.equal(rst.pformat(doc), expected(stem, 'tpxml'))
    assert.equal(rst.toXml(doc, { indents: true }), expected(stem, 'xml'))
  })
}
