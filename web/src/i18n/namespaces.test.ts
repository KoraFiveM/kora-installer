/// <reference types="node" />
// A file that uses a message must import the English catalogue file that registers it (`import '@/i18n/en/<area>/<file>'`):
// that is what puts the text in the same chunk as the code, and what makes the text exist at all when the file runs.
// This test reads the source and checks both ways: nothing used without its import, no import without a use (unless the line says `// i18n-dynamic`: ids that arrive from the server).
import { readFileSync } from 'node:fs'
import { relative, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import { sourceFiles } from './hardcoded.scan'

const ID = /'((?:core|errors|auth|servers|resources|backups|install|settings|system)\.[A-Za-z0-9]+\.[A-Za-z0-9.]+)'/g
const IMPORT = /^import '@\/i18n\/en\/([a-z]+\/[A-Za-z0-9]+)'$/gm

describe('catalogue imports', () => {
  const root = resolve(__dirname, '..')
  it('every file imports exactly the catalogue files whose messages it uses', () => {
    const problems: string[] = []
    for (const f of sourceFiles(root)) {
      const src = readFileSync(f, 'utf8')
      const used = new Set([...src.matchAll(ID)].map((m) => m[1]!.split('.').slice(0, 2).join('/')))
      const imported = new Set([...src.matchAll(IMPORT)].map((m) => m[1]!))
      for (const u of used) if (!imported.has(u)) problems.push(`${relative(root, f)}: uses ${u} but does not import '@/i18n/en/${u}'`)
      for (const i of imported) if (!used.has(i) && !new RegExp(`en/${i}'.*i18n-dynamic`).test(src)) problems.push(`${relative(root, f)}: imports '@/i18n/en/${i}' but uses none of its messages`)
    }
    expect(problems.join('\n')).toBe('')
  })
})
