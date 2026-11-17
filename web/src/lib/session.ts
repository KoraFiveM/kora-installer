import { ApiError, api } from '@/api/client'

/** `signed_out`: not signed in and nothing to try (show the sign-in screen, with no error). */
export type BootResult = { status: 'none' } | { status: 'signed_in' } | { status: 'signed_out' } | { status: 'failed'; error: ApiError } | { status: 'enrol'; link: string }

function fromHash(hash: string, key: string): string | null {
  const t = new URLSearchParams(hash.replace(/^#/, '')).get(key)
  return t && t.length > 0 ? t : null
}

/** Read `token=` from the URL hash (`#token=abc`), or null. */
export function tokenFromHash(hash: string): string | null {
  return fromHash(hash, 'token')
}

/** Read `code=` from the URL hash (`#code=abc`, a one-time sign-in link), or null. */
export function codeFromHash(hash: string): string | null {
  return fromHash(hash, 'code')
}

/** Read `enrol=` from the URL hash (`#enrol=abc`, a one-time enrolment link for a new user), or null. */
export function enrolFromHash(hash: string): string | null {
  return fromHash(hash, 'enrol')
}

/** Remove the hash from the address bar without adding a history entry or reloading. */
export function clearHash(): void {
  history.replaceState(null, '', window.location.pathname + window.location.search)
}

/**
 * Run once before the router starts. If the page was opened from a sign-in link (`/#code=<code>`, or the
 * older `/#token=<token>`), exchange it for a session and take it out of the address bar (even when the
 * exchange fails, so it never lingers). Otherwise, on another computer with no session, try the
 * Tailscale sign-in once: if this device's account is allowed, the person is simply in.
 */
export async function bootstrapSession(): Promise<BootResult> {
  // An enrolment link opens the set-up page, never a sign-in (it is not a credential for the session).
  const enrol = enrolFromHash(window.location.hash)
  if (enrol) {
    clearHash()
    return { status: 'enrol', link: enrol }
  }
  const code = codeFromHash(window.location.hash)
  const token = code ? null : tokenFromHash(window.location.hash)
  if (code || token) {
    clearHash()
    return code ? attempt(() => api.session.createByCode(code)) : signIn(token!)
  }
  return autoSignIn()
}

/** On a remote address with no session: one quiet try with the Tailscale identity. */
async function autoSignIn(): Promise<BootResult> {
  let meta
  try {
    meta = await api.meta()
  } catch {
    return { status: 'none' }
  }
  if (!meta.remote || meta.identity) return { status: 'none' }
  const r = await signInByIdentity()
  return r.status === 'signed_in' ? r : { status: 'signed_out' }
}

async function attempt(fn: () => Promise<void>): Promise<BootResult> {
  try {
    await fn()
    return { status: 'signed_in' }
  } catch (e) {
    return { status: 'failed', error: e instanceof ApiError ? e : new ApiError(0, { code: 'network', message: 'Could not sign in.' }) }
  }
}

/**
 * What was pasted in the sign-in box: a whole sign-in link (`http://.../#code=...` or `#token=...`) is
 * cut down to its code or token; anything else is sent as it is (the panel takes a code there too).
 */
export function pastedSecret(value: string): { code: string } | { token: string } {
  const v = value.trim()
  for (const key of ['code', 'token'] as const) {
    const at = v.indexOf(`#${key}=`)
    if (at >= 0) {
      const found = fromHash(v.slice(at), key)
      if (found) return key === 'code' ? { code: found } : { token: found }
    }
  }
  return { token: v }
}

/** Exchange what was pasted (a token, a code or a whole sign-in link) for a session. */
export async function signIn(token: string): Promise<BootResult> {
  const p = pastedSecret(token)
  return attempt(() => ('code' in p ? api.session.createByCode(p.code) : api.session.create(p.token)))
}

/** Sign in with no token: the panel asks Tailscale who is connecting (remote access only). */
export async function signInByIdentity(): Promise<BootResult> {
  return attempt(() => api.session.createByIdentity())
}
