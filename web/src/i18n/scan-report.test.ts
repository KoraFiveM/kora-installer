/// <reference types="node" />
import { it } from 'vitest'
import { resolve } from 'node:path'
import { scanAll } from './hardcoded.scan'
it('report', () => {
  const f = scanAll(resolve(__dirname, '..'))
  console.log('TOTAL', f.length)
  console.log(f.map((x) => `${x.file}:${x.line} ${x.text}`).join('\n'))
})
