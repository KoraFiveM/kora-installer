// Prints the size of what the browser loads at once (the files index.html refers to), raw and gzipped, and
// checks the production build does not contain the mock backend. Usage: node scripts/bundle-size.mjs [dist]
// Exits 1 when the mock is in the bundle, or when the first-load JS grows past the budget below.
import { readFileSync, readdirSync } from 'node:fs'
import { gzipSync } from 'node:zlib'
import { join } from 'node:path'

const dist = process.argv[2] ?? 'dist'
const BUDGET_GZIP_KB = 160 // first-load JS, gzipped. Was 191 kB in one file before the bundle was split.

const html = readFileSync(join(dist, 'index.html'), 'utf8')
const refs = [...html.matchAll(/(?:src|href)="\/(assets\/[^"]+\.(?:js|css))"/g)].map((m) => m[1])
const size = (f) => {
  const b = readFileSync(join(dist, f))
  return { raw: b.length, gz: gzipSync(b).length }
}
const kb = (n) => (n / 1000).toFixed(2).padStart(8) + ' kB'
let js = { raw: 0, gz: 0 }
let css = { raw: 0, gz: 0 }
for (const f of refs) {
  const s = size(f)
  const t = f.endsWith('.css') ? css : js
  t.raw += s.raw
  t.gz += s.gz
  console.log(`${kb(s.raw)}  ${kb(s.gz)} gzip  ${f}`)
}
console.log(`first-load JS : ${kb(js.raw)}  ${kb(js.gz)} gzip`)
console.log(`first-load CSS: ${kb(css.raw)}  ${kb(css.gz)} gzip`)

const all = readdirSync(join(dist, 'assets')).filter((f) => f.endsWith('.js'))
const total = all.reduce((a, f) => a + size(join('assets', f)).raw, 0)
console.log(`all JS files  : ${kb(total)} in ${all.length} files (the rest loads when a page or tab is first opened)`)

let bad = false
// Text that only the mock backend contains (fake data, banner). It must never ship.
const MOCK_MARKERS = ['Using the built-in mock backend', 'weekend-test', 'kora-mock-scenario']
for (const f of all) {
  const text = readFileSync(join(dist, 'assets', f), 'utf8')
  for (const m of MOCK_MARKERS) if (text.includes(m)) { console.error(`FAIL: "${m}" is in ${f}: the mock backend is in the production bundle`); bad = true }
}
if (js.gz / 1000 > BUDGET_GZIP_KB) { console.error(`FAIL: first-load JS is ${(js.gz / 1000).toFixed(1)} kB gzipped, over the ${BUDGET_GZIP_KB} kB budget`); bad = true }
process.exit(bad ? 1 : 0)
