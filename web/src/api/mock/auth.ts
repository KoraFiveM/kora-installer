// The mock of remote sign-in, API tokens and HTTPS: enough state to walk the screens (invite, enrol, sign in, devices, re-check)
// without a real panel. Passkey options are fake (the browser tests use Chromium's virtual authenticator for the real browser API);
// the "server" accepts what the page sends.
import type * as T from '../types'

type Fail = new (status: number, code: string, message: string, command?: string[], extra?: { details?: string; actions?: T.ProblemAction[] }) => Error
type Route = (method: string, pattern: string, h: (m: RegExpMatchArray, b: Record<string, unknown>, q: URLSearchParams) => unknown) => void

const now = () => Math.floor(Date.now() / 1000)

interface Account {
  /** The credential ids the browser made (base64url), so a sign-in can name them like the real panel does. */
  credentialIds: string[]
  passkeys: T.PasskeyView[]
  code: boolean
  recovery_left: number
  legacy_token: boolean
  devices: T.DeviceView[]
}

export interface AuthMockHooks {
  /** Who is looking, for the account screens. */
  viewer: () => { name: string; provider: string; remote: boolean }
  /** The users list of the remote settings (the mock keeps it in index.ts). */
  users: () => T.RemoteSettings['users']
  addUser: (name: string, role: T.Role) => void
  /** `true` once a reauth was done (for the mock's re-check scenario). */
  setSignedIn: (identity: T.Identity) => void
}

export function installAuthMock(route: Route, FailC: Fail, hooks: AuthMockHooks) {
  const Fail = FailC
  // Two invitations that exist from the start, for the browser tests (`/#enrol=demo-invite`).
  const links = new Map<string, { user: string; role: T.Role }>([
    ['demo-invite', { user: 'newbie', role: 'operator' }],
    ['demo-invite-codes', { user: 'amy', role: 'viewer' }],
  ])
  const accounts = new Map<string, Account>()
  const tokens: T.ApiTokenView[] = []
  let nextToken = 1
  let nextKey = 1
  const FRESH_KEY = 'kora-mock-fresh'
  const isFresh = () => { try { return window.sessionStorage.getItem(FRESH_KEY) !== '0' } catch { return true } }
  const setFresh = (v: boolean) => { try { window.sessionStorage.setItem(FRESH_KEY, v ? '1' : '0') } catch { /* private mode */ } }
  const acct = (u: string): Account => {
    if (!accounts.has(u)) {
      const legacy = (hooks.users().find((x) => x.name === u)?.legacy_token ?? false) || hooks.viewer().provider === 'token'
      accounts.set(u, { credentialIds: [], passkeys: [], code: false, recovery_left: 0, legacy_token: legacy, devices: [{ handle: 'dev-this', device: 'Chrome on Linux', address: '192.168.1.20', signed_in: now() - 600, last_seen: now(), current: true }] })
    }
    return accounts.get(u)!
  }
  const hex = () => Array.from(crypto.getRandomValues(new Uint8Array(16)), (b) => b.toString(16).padStart(2, '0')).join('')
  const sync = (name: string) => {
    const a = acct(name)
    const u = hooks.users().find((x) => x.name === name)
    if (u) Object.assign(u, { passkeys: a.passkeys.length, code: a.code, legacy_token: a.legacy_token })
  }
  // A passkey needs a NAME; on a bare address the browser refuses it, as it does for real.
  const rpId = location.hostname
  const options = (user: string, ids: string[] = []) => ({
    publicKey: {
      challenge: 'AAAA',
      rp: { id: rpId, name: 'Kora Panel' },
      rpId,
      user: { id: 'AAAA', name: user, displayName: user },
      pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
      authenticatorSelection: { residentKey: 'preferred', userVerification: 'preferred' },
      ...(ids.length ? { allowCredentials: ids.map((id) => ({ type: 'public-key', id })) } : {}),
      timeout: 60000,
    },
  })
  const rawIdOf = (b: Record<string, unknown>) => String((b.credential as { rawId?: string } | undefined)?.rawId ?? '')
  const qr = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10" role="img" aria-label="QR code for the authenticator app"><rect width="10" height="10" fill="#fff"/><rect x="1" y="1" width="3" height="3"/><rect x="6" y="1" width="3" height="3"/><rect x="1" y="6" width="3" height="3"/></svg>'

  route('GET', '/auth/options', () => ({ passkey: true, passkey_problem: null, code: true, token: true, tailscale: false, strong_required: false }) satisfies T.AuthOptions)
  route('POST', '/auth/login', (_m, b) => {
    const step = b.step as string
    const who = hooks.viewer()
    if (step === 'passkey_begin') {
      if (b.user === 'nobody') throw new Fail(401, 'unauthorized', 'There is no passkey for that name on this address. Use the name you registered it with, sign in with a one-time code, or ask an administrator for a new enrolment link.')
      return { step: 'challenge', handle: 'h1', options: options(String(b.user), acct(String(b.user)).credentialIds) } satisfies T.LoginResponse
    }
    if (step === 'code' && b.password === 'wrong') throw new Fail(401, 'unauthorized', 'That name, password or code is not right (or the account is locked for a few minutes). Try again, or use a passkey.')
    const identity: T.Identity = { name: String(b.user ?? who.name), provider: step === 'code' ? 'code' : 'passkey', role: 'admin' }
    hooks.setSignedIn(identity)
    return { step: 'signed_in', identity, secret: 'mock-session-secret', notice: null } satisfies T.LoginResponse
  })
  route('POST', '/auth/enrol', (_m, b) => {
    const link = String(b.link ?? '')
    const l = links.get(link)
    if (!l) throw new Fail(401, 'unauthorized', 'This link has expired or was already used. Ask the person who runs Kora for a new one.')
    switch (b.step) {
      case 'check':
        return { step: 'who', user: l.user, role: l.role, passkey: true, passkey_problem: null } satisfies T.EnrolResponse
      case 'passkey_begin':
        return { step: 'challenge', handle: 'e1', options: options(l.user) } satisfies T.EnrolResponse
      case 'passkey_finish': {
        links.delete(link)
        const a = acct(l.user)
        a.credentialIds.push(rawIdOf(b))
        a.passkeys.push({ id: nextKey++, label: String(b.label || 'Passkey'), rp_id: rpId, created: now(), last_used: 0 })
        sync(l.user)
        const identity: T.Identity = { name: l.user, provider: 'passkey', role: l.role }
        hooks.setSignedIn(identity)
        return { step: 'done', identity, secret: 'mock-session-secret', recovery_codes: [] } satisfies T.EnrolResponse
      }
      case 'code_begin':
        return { step: 'code_setup', secret: 'JBSWY3DPEHPK3PXP', otpauth: `otpauth://totp/Kora%20Panel:${l.user}?secret=JBSWY3DPEHPK3PXP&issuer=Kora%20Panel`, qr_svg: qr } satisfies T.EnrolResponse
      case 'code_finish': {
        if (String(b.password ?? '').length < 12) throw new Fail(400, 'bad_request', 'Use at least 12 characters. A few random words in a row make a strong, easy-to-remember password.')
        if (!/^\d{6}$/.test(String(b.code ?? '').replace(/\s/g, ''))) throw new Fail(400, 'bad_request', 'That code is not right. Check the time on your phone, wait for the next code, and try again.')
        links.delete(link)
        const a = acct(l.user)
        a.code = true
        a.recovery_left = 10
        sync(l.user)
        const identity: T.Identity = { name: l.user, provider: 'code', role: l.role }
        hooks.setSignedIn(identity)
        return { step: 'done', identity, secret: 'mock-session-secret', recovery_codes: Array.from({ length: 10 }, (_, i) => `mock${i}-abcd${i}`) } satisfies T.EnrolResponse
      }
    }
    throw new Fail(400, 'bad_request', 'Unknown step.')
  })
  route('POST', '/auth/reauth', (_m, b) => {
    if (b.step === 'passkey_begin') return { step: 'challenge', handle: 'r1', options: options(hooks.viewer().name, acct(hooks.viewer().name).credentialIds) } satisfies T.ReauthResponse
    if (b.step === 'code' && b.password === 'wrong') throw new Fail(401, 'unauthorized', 'That name, password or code is not right (or the account is locked for a few minutes). Try again, or use a passkey.')
    setFresh(true)
    return { step: 'fresh', valid_for: 600 } satisfies T.ReauthResponse
  })
  route('GET', '/auth/account', () => {
    const v = hooks.viewer()
    const a = acct(v.name)
    return {
      user: v.name,
      provider: v.provider as T.Identity['provider'],
      role: 'admin',
      passkeys: structuredClone(a.passkeys),
      code: a.code,
      recovery_left: a.recovery_left,
      legacy_token: a.legacy_token,
      strong: a.passkeys.length > 0 || a.code,
      passkey_problem: null,
      upgrade_suggested: v.provider === 'token' && a.passkeys.length === 0 && !a.code,
      devices: structuredClone(a.devices),
    } satisfies T.AccountView
  })
  route('POST', '/auth/account', (_m, b) => {
    const v = hooks.viewer()
    const a = acct(v.name)
    const strong = a.passkeys.length > 0 || a.code
    const protective = b.step === 'device_revoke' || b.step === 'devices_revoke_others'
    if (strong && !protective && !isFresh()) throw new Fail(403, 'remote.reauth_required', 'Confirm it is you first: use your passkey or a one-time code, then try again.')
    switch (b.step) {
      case 'passkey_begin':
        return { step: 'challenge', handle: 'a1', options: options(v.name) } satisfies T.AccountChanged
      case 'passkey_finish':
        a.credentialIds.push(rawIdOf(b))
        a.passkeys.push({ id: nextKey++, label: String(b.label || 'Passkey'), rp_id: rpId, created: now(), last_used: 0 })
        sync(v.name)
        return { step: 'done' } satisfies T.AccountChanged
      case 'passkey_remove':
        if (a.passkeys.length === 1 && !a.code) throw new Fail(409, 'conflict', 'This is your only way to sign in. Add another passkey or set up one-time codes first, then remove this one.')
        a.passkeys = a.passkeys.filter((p) => p.id !== b.id)
        sync(v.name)
        return { step: 'done' } satisfies T.AccountChanged
      case 'code_begin':
        return { step: 'code_setup', secret: 'JBSWY3DPEHPK3PXP', otpauth: 'otpauth://totp/Kora%20Panel:' + v.name, qr_svg: qr } satisfies T.AccountChanged
      case 'code_finish':
        a.code = true
        a.recovery_left = 10
        sync(v.name)
        return { step: 'recovery', codes: Array.from({ length: 10 }, (_, i) => `mock${i}-abcd${i}`) } satisfies T.AccountChanged
      case 'password_change':
        return { step: 'done' } satisfies T.AccountChanged
      case 'recovery_new':
        a.recovery_left = 10
        return { step: 'recovery', codes: Array.from({ length: 10 }, (_, i) => `new${i}-abcd${i}`) } satisfies T.AccountChanged
      case 'device_revoke':
        a.devices = a.devices.filter((d) => d.handle !== b.handle)
        return { step: 'done' } satisfies T.AccountChanged
      case 'devices_revoke_others':
        a.devices = a.devices.filter((d) => d.current)
        return { step: 'done' } satisfies T.AccountChanged
      case 'token_revoke':
        if (!strong) throw new Fail(409, 'conflict', 'Set up a passkey or one-time codes first, so you do not lock yourself out.')
        a.legacy_token = false
        sync(v.name)
        return { step: 'done' } satisfies T.AccountChanged
    }
    throw new Fail(400, 'bad_request', 'Unknown step.')
  })
  // The computer's own controls.
  route('POST', '/users/{name}/reset', (m, b) => {
    const name = decodeURIComponent(m[1])
    if (!hooks.users().some((u) => u.name === name)) throw new Fail(404, 'not_found', `There is no user named '${name}'.`)
    const a = acct(name)
    const what = b.what as T.ResetWhat
    if (what === 'passkeys' || what === 'all') a.passkeys = []
    if (what === 'code' || what === 'all') {
      a.code = false
      a.recovery_left = 0
    }
    if (what === 'legacy_token' || what === 'all') a.legacy_token = false
    sync(name)
    if (what === 'legacy_token') return { name, link: null, expires_in: 1800 } satisfies T.ResetDone
    const link = hex()
    links.set(link, { user: name, role: 'viewer' })
    return { name, link, expires_in: 1800 } satisfies T.ResetDone
  })
  route('GET', '/users/{name}/credentials', (m) => {
    const name = decodeURIComponent(m[1])
    const u = hooks.users().find((x) => x.name === name)
    if (!u) throw new Fail(404, 'not_found', `There is no user named '${name}'.`)
    const a = acct(name)
    return { name, role: u.role, passkeys: structuredClone(a.passkeys), code: a.code, recovery_left: a.recovery_left, legacy_token: a.legacy_token, signed_in_devices: a.devices.length } satisfies T.UserCredentials
  })
  route('GET', '/api-tokens', () => ({ tokens: structuredClone(tokens) }))
  route('POST', '/api-tokens', (_m, b) => {
    const name = String(b.name ?? '').trim()
    if (!/^[A-Za-z0-9._-]{1,40}$/.test(name)) throw new Fail(400, 'bad_request', 'A token name uses letters, numbers, dots, dashes and underscores, up to 40 characters (for example backup-script).')
    if (tokens.some((t) => t.name === name)) throw new Fail(409, 'conflict', `There is already an API token named '${name}'.`)
    const role = b.role as T.Role
    const days = typeof b.days === 'number' ? b.days : null
    const t: T.ApiTokenView = { id: nextToken++, name, role, created: now(), expires: days ? now() + days * 86400 : 0, last_used: 0 }
    tokens.push(t)
    return { id: t.id, name, role, token: 'kora_' + hex() + hex() } satisfies T.NewApiToken
  })
  route('DELETE', '/api-tokens/{id}', (m) => {
    const id = Number(m[1])
    const i = tokens.findIndex((t) => t.id === id)
    if (i < 0) throw new Fail(404, 'not_found', 'There is no such API token.')
    tokens.splice(i, 1)
    return undefined
  })
  // HTTPS.
  let tls: T.TlsStatus = {
    mode: 'self-signed',
    this_connection_secure: false,
    active: true,
    source: 'self-signed',
    names: ['localhost', 'box.lan', '192.168.1.5'],
    not_after: now() + 300 * 86400,
    days_left: 300,
    fingerprint: 'AB:12:CD:34:EF:56:78:90:AB:12:CD:34:EF:56:78:90:AB:12:CD:34:EF:56:78:90:AB:12:CD:34:EF:56:78:90',
    ca_fingerprint: '11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00',
    error: null,
    cert_path: null,
    proxy_trusted: false,
  }
  route('GET', '/tls', () => structuredClone(tls))
  route('POST', '/tls/renew', () => {
    tls = { ...tls, not_after: now() + 397 * 86400, days_left: 397 }
    return structuredClone(tls)
  })
  route('GET', '/tls/ca', () => ({ pem: '-----BEGIN CERTIFICATE-----\nMIIBmock\n-----END CERTIFICATE-----\n', fingerprint: tls.ca_fingerprint ?? '' }) satisfies T.TlsCa)

  return {
    hasStrong(name: string) {
      const a = acct(name)
      return a.passkeys.length > 0 || a.code
    },
    /** The link secret for a person just invited, so the invite screen and the enrol screen agree. */
    mintLink(user: string, role: T.Role): string {
      const link = hex()
      links.set(link, { user, role })
      return link
    },
    /** Scenario helpers for the browser tests. */
    setTls(p: Partial<T.TlsStatus>) {
      tls = { ...tls, ...p }
    },
    needFresh(v: boolean) {
      setFresh(!v)
    },
    reset() {
      links.clear()
      accounts.clear()
      tokens.length = 0
    },
  }
}
