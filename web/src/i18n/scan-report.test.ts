/// <reference types="node" />
import { it } from 'vitest'
import { resolve } from 'node:path'
import { scanAll } from './hardcoded.scan'
it('report', () => {
  const f = scanAll(resolve(__dirname, '..'))
  const by = new Map<string, number>()
  for (const x of f) by.set(x.file, (by.get(x.file) ?? 0) + 1)
  console.log('TOTAL', f.length, 'in', by.size, 'files')
  console.log([...by].sort((a, b) => b[1] - a[1]).map(([k, v]) => `${v}\t${k}`).join('\n'))
  if (process.env.SHOW) console.log(f.filter((x) => x.file.includes(process.env.SHOW!)).map((x) => `${x.file}:${x.line} [${x.kind}] ${x.text}`).join('\n'))
})
