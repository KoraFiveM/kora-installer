/// <reference types="node" />
// The message ids the Rust side sends (`Problem::with_message_id("errors.problems.x", ...)`) must exist in the English catalogue (and in
// every other language): the id is the contract between crates/ and this panel.
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import { catalogue } from './index'
import { args, parse } from './icu'
import './de'
import.meta.glob('./en/errors/*.ts', { eager: true })

function rustFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((e) => {
    const p = join(dir, e)
    return statSync(p).isDirectory() ? rustFiles(p) : p.endsWith('.rs') ? [p] : []
  })
}

describe('message ids sent by the server', () => {
  const crates = resolve(__dirname, '../../../crates')
  const sent = new Map<string, string[]>()
  for (const f of rustFiles(crates)) {
    for (const m of readFileSync(f, 'utf8').matchAll(/with_message_id\(\s*"([^"]+)"\s*,\s*&\[([^\]]*)\]/g)) {
      const params = [...m[2]!.matchAll(/\("([a-z_A-Z]+)"\s*,/g)].map((x) => x[1]!)
      sent.set(m[1]!, params)
    }
  }

  it('finds them', () => {
    expect(sent.size).toBeGreaterThanOrEqual(10)
  })

  it('every id exists in English and German, takes the parameters the server sends, and is under errors.problems.', () => {
    const en = catalogue('en')
    const de = catalogue('de')
    const problems: string[] = []
    for (const [id, params] of sent) {
      if (!id.startsWith('errors.problems.')) problems.push(`${id}: server message ids live under errors.problems.`)
      for (const [name, cat] of [['en', en], ['de', de]] as const) {
        const text = cat[id]
        if (text === undefined) {
          problems.push(`${id}: missing in ${name}`)
          continue
        }
        const wants = [...args(parse(text)).keys()].sort().join(',')
        if (wants !== [...params].sort().join(',')) problems.push(`${id}: the server sends [${params.join(',')}], ${name} message uses [${wants}]`)
      }
    }
    expect(problems.join('\n')).toBe('')
  })
})
