/// <reference types="node" />
// Fails when user-facing text is written straight into the source instead of the catalogue (src/i18n/en/*.ts).
// What counts, and how to make an exception (`// i18n-ignore: why`): see the header of hardcoded.scan.ts.
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import { scanAll, scanSource } from './hardcoded.scan'

const flagged = (code: string) => scanSource('x.tsx', code).map((f) => f.text)

describe('the scanner itself', () => {
  it('flags text between tags, in attributes and in expressions', () => {
    expect(flagged('const a = <p>Save your changes</p>')).toEqual(['Save your changes'])
    expect(flagged('const a = <button aria-label="Close this dialog" />')).toEqual(['Close this dialog'])
    expect(flagged('const a = <input placeholder="Search" />')).toEqual(['Search'])
    expect(flagged("const a = <p>{ok ? 'Saved' : 'Not saved'}</p>")).toEqual(['Saved', 'Not saved'])
    expect(flagged('const a = <p title={`Delete ${name}?`} />')).toEqual(['Delete ?'])
  })
  it('flags sentences and labels in plain code, and fragments of sentences', () => {
    expect(flagged("function f() { return 'Something went wrong. Try again.' }")).toEqual(['Something went wrong. Try again.'])
    expect(flagged("const T = { running: 'Running' }")).toEqual(['Running'])
    expect(flagged("function f(n: number) { return `${n} was added.` }")).toEqual(['was added.'])
  })
  it('leaves code alone', () => {
    expect(
      flagged(`
        import x from 'some package'
        const a = <div className="flex items-center gap-2" id="main" role="status" href="/settings"><Icon aria-hidden /></div>
        const b = { kind: 'running', to: '/settings' }
        if (state === 'not allowed') {}
        console.log('Something went wrong here')
        localStorage.getItem('kora panel session')
        const c = t('core.skip')
        const d = <p>{t('core.skip')}</p>
        const e = <p>Kora</p>
        const f = <p>{count}</p>
      `),
    ).toEqual([])
  })
  it('honours i18n-ignore', () => {
    expect(flagged("const x = 'Only for developers to read' // i18n-ignore: log")).toEqual([])
    expect(flagged("// i18n-ignore: a command\nconst x = 'Run this command now'")).toEqual([])
  })
})

describe('the app', () => {
  it('has no user-facing text outside the catalogue', () => {
    const found = scanAll(resolve(__dirname, '..'))
    const text = found.map((f) => `  ${f.file}:${f.line} [${f.kind}] ${f.text}`).join('\n')
    expect(text, `Text written into the source. Put it in src/i18n/en/<area>.ts and use t('id'); a deliberate exception takes "// i18n-ignore: why":\n${text}\n`).toBe('')
  }, 60_000)
})
