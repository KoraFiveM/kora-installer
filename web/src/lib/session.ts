import { ApiError, api } from '@/api/client'

export type BootResult = { status: 'none' } | { status: 'signed_in' } | { status: 'failed'; error: ApiError }

/** Read `token=` from the URL hash (`#token=abc`), or null. */
export function tokenFromHash(hash: string): string | null {
  const t = new URLSearchParams(hash.replace(/^#/, '')).get('token')
  return t && t.length > 0 ? t : null
}

/** Remove the hash from the address bar without adding a history entry or reloading. */
export function clearHash(): void {
  history.replaceState(null, '', window.location.pathname + window.location.search)
}

/**
 * Run once before the router starts. If the page was opened as `/#token=<token>`, exchange the token for the
 * session cookie and take it out of the address bar (even when the exchange fails, so it never lingers).
 */
export async function bootstrapSession(): Promise<BootResult> {
  const token = tokenFromHash(window.location.hash)
  if (!token) return { status: 'none' }
  clearHash()
  return signIn(token)
}

/** Exchange a token for a session (also used by the "paste a token" field). */
export async function signIn(token: string): Promise<BootResult> {
  try {
    await api.session.create(token.trim())
    return { status: 'signed_in' }
  } catch (e) {
    return { status: 'failed', error: e instanceof ApiError ? e : new ApiError(0, { code: 'network', message: 'Could not sign in.' }) }
  }
}
