// Mock data and routes for the saved licence keys, the key of an installed server and the remembered profile
// (kept apart from the main mock so edits do not collide). `installKeysMock` is called once from mock/index.ts.
// Like the real panel it never hands out a full key: only the name and the last four characters.
import type * as T from '../types'

type Body = Record<string, unknown>
type Route = (method: string, pattern: string, h: (m: RegExpMatchArray, body: Body, query: URLSearchParams) => unknown | Promise<unknown>) => void
type FailCtor = new (status: number, code: string, message: string, command?: string[], extra?: Partial<T.JobError>) => Error

interface Stored {
  name: string
  key: string
  added: number
}

export interface KeysMock {
  /** Back to the start of the demo (called whenever the scenario is (re)loaded). */
  reset: (opts: { trouble: boolean; withKeys: boolean }) => void
  /** An install request: check a named key exists, save a pasted one when asked. Throws like the panel does. */
  forInstall: (req: T.InstallRequest) => void
  /** A Cfx.re account was found / a GitHub owner was used: remembered for next time. */
  remember: (p: Partial<T.Profile>) => void
  profile: () => T.Profile
  /** Name of a saved key the saved answers point to that is gone (for the wizard to say so). */
  has: (name: string) => boolean
}

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase()
const last4 = (k: string) => k.slice(-4)

export function installKeysMock(route: Route, Fail: FailCtor, servers: () => T.ServerSummary[], canManage: () => boolean = () => true): KeysMock {
  let keys: Stored[] = []
  let trouble = false
  let profile: T.Profile = { cfx_username: null, cfx_identifier: null, github_owner: null }
  /** Which saved key each demo server uses (by name), set when a key is applied to it. */
  let usedBy: Record<string, string> = {}

  const need = () => {
    if (trouble) {
      throw new Fail(500, 'keys.unreadable', 'Kora cannot read your saved licence keys (the file is damaged). Nothing was changed and no key was lost: the file is still where it was. Fix or move the file /home/demo/.kora/secrets.json and try again.', undefined, { actions: [{ kind: 'copy_details' }] })
    }
  }
  const checkName = (raw: unknown): string => {
    const n = String(raw ?? '').trim()
    if (!n) throw new Fail(400, 'keys.invalid', 'Give the key a name you will recognise, for example "Main" or "Test server".')
    if (n.length > 40) throw new Fail(400, 'keys.invalid', 'That name is too long. Use up to 40 characters.')
    return n
  }
  const checkKey = (raw: unknown): string => {
    const k = String(raw ?? '').trim()
    if (!k) throw new Fail(400, 'keys.invalid', 'Paste the licence key first. You can get one at https://portal.cfx.re/servers/registration-keys.')
    if (!/^[A-Za-z0-9_-]{8,64}$/.test(k)) throw new Fail(400, 'keys.invalid', 'That does not look like a Cfx.re licence key. It is 8 to 64 letters, numbers, dashes or underscores, with no spaces. Copy it again from portal.cfx.re.')
    return k
  }
  const find = (name: string) => keys.find((k) => same(k.name, name))
  const missing = (name: string) => new Fail(404, 'keys.not_found', `There is no saved licence key called "${name}" (it may have been renamed or deleted). Pick another saved key or add the key again.`, undefined, { actions: [{ kind: 'change_answers', section: 'licence' }] })
  const add = (nameRaw: unknown, keyRaw: unknown): Stored => {
    const name = checkName(nameRaw)
    const key = checkKey(keyRaw)
    const taken = find(name)
    if (taken) throw new Fail(409, 'keys.name_taken', `A saved key is already called "${taken.name}". Choose another name.`)
    const dup = keys.find((k) => k.key === key)
    if (dup) throw new Fail(409, 'keys.duplicate', `That key is already saved as "${dup.name}". Pick it from the list instead of adding it again.`)
    const s = { name, key, added: Math.floor(Date.now() / 1000) }
    keys.push(s)
    return s
  }
  const info = (k: Stored): T.SavedKeyInfo => ({ name: k.name, last4: last4(k.key), added: k.added })

  route('GET', '/keys', () => {
    need()
    return {
      keys: keys.map((k) => ({ ...info(k), servers: Object.entries(usedBy).filter(([s, n]) => same(n, k.name) && servers().some((x) => x.name === s)).map(([s]) => s), stopped_installs: 0 })),
      can_manage: canManage(),
      path: '/home/demo/.kora/secrets.json',
    } satisfies T.SavedKeys
  })
  route('POST', '/keys', (_m, b) => {
    need()
    return { key: info(add(b.name, b.license_key)) }
  })
  route('PUT', '/keys/{name}', (m, b) => {
    need()
    const cur = find(decodeURIComponent(m[1]))
    if (!cur) throw missing(decodeURIComponent(m[1]))
    if (b.new_name === undefined && b.license_key === undefined) throw new Fail(400, 'bad_request', 'Give a new name or a new key.')
    const newName = b.new_name === undefined ? null : checkName(b.new_name)
    const newKey = b.license_key === undefined ? null : checkKey(b.license_key)
    if (newName && keys.some((k) => k !== cur && same(k.name, newName))) throw new Fail(409, 'keys.name_taken', `A saved key is already called "${newName}". Choose another name.`)
    const dup = newKey ? keys.find((k) => k !== cur && k.key === newKey) : undefined
    if (dup) throw new Fail(409, 'keys.duplicate', `That key is already saved as "${dup.name}". Pick it from the list instead of adding it again.`)
    const old = cur.name
    if (newName) {
      cur.name = newName
      for (const s of Object.keys(usedBy)) if (same(usedBy[s], old)) usedBy[s] = newName
    }
    if (newKey) cur.key = newKey
    const notes = newKey ? ['Servers that were already installed with the old key keep using it. Use "Change licence key" on a server to switch it.'] : []
    return { key: info(cur), notes } satisfies T.KeyChange
  })
  route('DELETE', '/keys/{name}', (m) => {
    need()
    const cur = find(decodeURIComponent(m[1]))
    if (!cur) throw missing(decodeURIComponent(m[1]))
    keys = keys.filter((k) => k !== cur)
    return { removed: true, notes: ['Servers that were installed with this key keep working: they have their own copy.'] }
  })
  route('PUT', '/servers/{s}/licence-key', (m, b) => {
    const name = decodeURIComponent(m[1])
    const s = servers().find((x) => x.name === name)
    if (!s) throw new Fail(404, 'not_found', `There is no server called "${name}".`)
    const picked = typeof b.name === 'string' ? b.name : null
    const pasted = typeof b.license_key === 'string' ? b.license_key : null
    if ((picked === null) === (pasted === null)) throw new Fail(400, 'bad_request', 'Pick one of the saved keys, or paste a new key (not both).')
    let key: Stored | null
    let lastFour: string
    if (picked !== null) {
      need()
      key = find(picked) ?? null
      if (!key) throw missing(picked)
      lastFour = last4(key.key)
    } else {
      const k = checkKey(pasted)
      lastFour = last4(k)
      key = keys.find((x) => x.key === k) ?? null
      if (!key && typeof b.save_as === 'string' && b.save_as.trim()) key = add(b.save_as, k)
    }
    if (key) usedBy[s.name] = key.name
    else delete usedBy[s.name]
    const running = s.status === 'running'
    return {
      server: s.name,
      last4: lastFour,
      key_name: key?.name ?? null,
      running,
      restart_needed: running,
      message: running
        ? "The new licence key is saved in the server's configuration. The server only reads it when it starts, so restart it to switch over."
        : "The new licence key is saved in the server's configuration. No restart is needed: the server is stopped and will use it the next time it starts.",
    } satisfies T.ServerKeyChanged
  })
  route('GET', '/profile', () => ({ ...profile }))
  route('PUT', '/profile', (_m, b) => {
    const pick = (k: keyof T.Profile) => (b[k] === undefined ? profile[k] : b[k] === null || String(b[k]).trim() === '' ? null : String(b[k]).trim())
    profile = { cfx_username: pick('cfx_username'), cfx_identifier: pick('cfx_identifier'), github_owner: pick('github_owner') }
    return { ...profile }
  })

  return {
    reset: (o) => {
      trouble = o.trouble
      profile = { cfx_username: null, cfx_identifier: null, github_owner: null }
      usedBy = {}
      keys = o.withKeys ? [{ name: 'Main', key: 'cfxk_DEMOKEYMAIN1234567_4f2a', added: 1_760_000_000 }, { name: 'Test server', key: 'cfxk_DEMOKEYTEST1234567_9c1e', added: 1_761_000_000 }] : []
    },
    forInstall: (req) => {
      if (req.license_key_name) {
        need()
        if (!find(req.license_key_name)) throw missing(req.license_key_name)
        return
      }
      const name = req.save_license_key_as?.trim()
      if (req.license_key && name) {
        need()
        const existing = keys.find((k) => k.key === req.license_key!.trim())
        if (existing) req.license_key_name = existing.name
        else req.license_key_name = add(name, req.license_key).name
      }
    },
    remember: (p) => {
      profile = { ...profile, ...p }
    },
    profile: () => ({ ...profile }),
    has: (name) => !!find(name),
  }
}
