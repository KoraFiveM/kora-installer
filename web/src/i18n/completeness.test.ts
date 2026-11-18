// Every language besides English is checked against English (the source of truth): nothing missing, nothing extra, the same
// {arguments} and <tags> (a translation may reorder them, not drop or invent them), valid syntax, and no English left over.
import { describe, expect, it } from 'vitest'
import { args, parse } from './icu'
import { catalogue } from './index'
import { LOCALES } from './locales'
import './de'

// Every English catalogue file registers itself when imported.
import.meta.glob('./en/**/*.ts', { eager: true })

/** English words that stay as they are in any language: products, protocols and technical words. */
const SAME = /^(Kora|txAdmin|FiveM|GitHub|Cfx\.re|Tailscale|Discord|Windows|Linux|macOS|HTTPS?|TLS|UPnP|PCP|NAT-PMP|SQLite|MariaDB|MySQL|API|URL|IP|OK|ID|CPU|RAM|GB|MB|KB|TCP|UDP|CLI|PR|SHA|Git|PowerShell|Docker|Cloudflare|PEM|CA|SHA-256|JSON|ssh|sudo|root|iPhone|iPad|Android|Safari|Chrome|Chromium|Firefox|Edge|Server|Port|Ports|Name|Status|Token|Tokens|Backups?|Resources?|Logs?|Fork|Forks|Branch|Branches|Commit|Commits|Pull|Request|Requests|Issues?|Releases?|Snapshots?|Props|Admin|Operator|Updates?|Passkeys?|Host|Test|Build|Version|Fleet|Hostname|Webhook|Proxy|Update|Stash|Escrow|ensure|Main|Info|Details?|Firewall|Online|Nodes?|Problem|Download|Fleet|Passwort|Beta)$/

const words = (s: string) => s.replace(/\{[^}]*\}|<\/?[A-Za-z0-9]+>|\S*[/:]\S*/g, ' ').match(/[A-Za-z][A-Za-z'’.-]*/g) ?? []

describe.each(LOCALES.filter((l) => l.code !== 'en' && !l.dev))('$name', (loc) => {
  const en = catalogue('en')
  const tr = catalogue(loc.code)

  it('has every message of English, and no others', () => {
    const missing = Object.keys(en).filter((id) => !(id in tr))
    const extra = Object.keys(tr).filter((id) => !(id in en))
    expect({ missing, extra }).toEqual({ missing: [], extra: [] })
  })

  it('every message is valid and uses the same arguments and tags as English', () => {
    const bad: string[] = []
    for (const [id, text] of Object.entries(tr)) {
      if (!(id in en)) continue
      let want: Map<string, string>
      let got: Map<string, string>
      try {
        want = args(parse(en[id]!))
        got = args(parse(text))
      } catch (e) {
        bad.push(`${id}: ${(e as Error).message}`)
        continue
      }
      const a = [...want].map(([n, k]) => `${n}:${k}`).sort().join(' ')
      const b = [...got].map(([n, k]) => `${n}:${k}`).sort().join(' ')
      if (a !== b) bad.push(`${id}: English has [${a}], ${loc.code} has [${b}]`)
    }
    expect(bad.join('\n')).toBe('')
  })

  it('no English is left in it (a message that equals the English one must be only names or technical words)', () => {
    const left: string[] = []
    for (const [id, text] of Object.entries(tr)) {
      if (en[id] !== text) continue
      const w = words(text).filter((x) => !SAME.test(x))
      if (w.length > 0) left.push(`${id}: ${text.slice(0, 70)}`)
    }
    expect(left.join('\n')).toBe('')
  })

  it('keeps the commands and file names that the English shows in <code> or backticks', () => {
    const bad: string[] = []
    for (const [id, text] of Object.entries(en)) {
      const t = tr[id]
      if (!t) continue
      for (const m of text.matchAll(/<code\d?>([^<]*)<\/code\d?>|`([^`]+)`/g)) {
        const lit = (m[1] ?? m[2])!.replace(/^\{.*\}$/, '')
        if (lit && !t.includes(lit)) bad.push(`${id}: "${lit}" is not in the translation`)
      }
    }
    expect(bad.join('\n')).toBe('')
  })
})
