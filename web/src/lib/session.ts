import { ApiError, api } from '@/api/client'

/** `signed_out`: not signed in and nothing to try (show the sign-in screen, with no error). */
export type BootResult = { status: 'none' } | { status: 'signed_in' } | { status: 'signed_out' } | { status: 'failed'; error: ApiError }

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

/** Exchange a token for a session (also used by the "paste a token" field). */
export async function signIn(token: string): Promise<BootResult> {
  return attempt(() => api.session.create(token.trim()))
}

/** Sign in with no token: the panel asks Tailscale who is connecting (remote access only). */
export async function signInByIdentity(): Promise<BootResult> {
  return attempt(() => api.session.createByIdentity())
}
