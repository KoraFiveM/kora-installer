import { t } from '@/i18n'
import '@/i18n/en/core/webauthn'
// Passkeys in the browser: the panel sends WebAuthn options as JSON with base64url strings, the browser API wants
// ArrayBuffers, and the answer goes back as JSON with base64url strings again (what the Rust side, webauthn-rs, reads).
// Nothing here is cryptography: the browser and the authenticator do it all.

export function b64urlToBytes(s: string): Uint8Array<ArrayBuffer> {
  const pad = '='.repeat((4 - (s.length % 4)) % 4)
  const bin = atob(s.replace(/-/g, '+').replace(/_/g, '/') + pad)
  const out = new Uint8Array(new ArrayBuffer(bin.length))
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i)
  return out
}

export function bytesToB64url(b: ArrayBuffer | Uint8Array): string {
  const bytes = b instanceof Uint8Array ? b : new Uint8Array(b)
  let s = ''
  for (const x of bytes) s += String.fromCharCode(x)
  return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

/** Can this page use passkeys at all (a secure address, and a browser that has them)? */
export function passkeysSupported(): boolean {
  return typeof window !== 'undefined' && window.isSecureContext && typeof window.PublicKeyCredential !== 'undefined' && !!navigator.credentials
}

interface Descriptor {
  id: string
  [k: string]: unknown
}
interface PublicKeyOptions {
  challenge: string
  user?: { id: string; [k: string]: unknown }
  excludeCredentials?: Descriptor[]
  allowCredentials?: Descriptor[]
  [k: string]: unknown
}

function revive(o: PublicKeyOptions): PublicKeyCredentialCreationOptions & PublicKeyCredentialRequestOptions {
  const out = { ...o, challenge: b64urlToBytes(o.challenge) } as Record<string, unknown>
  if (o.user) out.user = { ...o.user, id: b64urlToBytes(o.user.id) }
  if (o.excludeCredentials) out.excludeCredentials = o.excludeCredentials.map((c) => ({ ...c, id: b64urlToBytes(c.id) }))
  if (o.allowCredentials) out.allowCredentials = o.allowCredentials.map((c) => ({ ...c, id: b64urlToBytes(c.id) }))
  return out as unknown as PublicKeyCredentialCreationOptions & PublicKeyCredentialRequestOptions
}

function publicKey(options: unknown): PublicKeyOptions {
  const pk = (options as { publicKey?: PublicKeyOptions } | null)?.publicKey
  if (!pk) throw new Error(t('core.webauthn.panelDidSendPasskey'))
  return pk
}

/** Register a passkey: the answer for `finish` steps (`credential`). */
export async function createPasskey(options: unknown): Promise<unknown> {
  const cred = (await navigator.credentials.create({ publicKey: revive(publicKey(options)) })) as PublicKeyCredential | null
  if (!cred) throw new Error(t('core.webauthn.noPasskeyMade'))
  const r = cred.response as AuthenticatorAttestationResponse
  return {
    id: cred.id,
    rawId: bytesToB64url(cred.rawId),
    type: cred.type,
    extensions: cred.getClientExtensionResults(),
    response: { attestationObject: bytesToB64url(r.attestationObject), clientDataJSON: bytesToB64url(r.clientDataJSON), transports: r.getTransports?.() ?? [] },
  }
}

/** Sign with a passkey: the answer for `finish` steps (`credential`). */
export async function getPasskey(options: unknown): Promise<unknown> {
  const cred = (await navigator.credentials.get({ publicKey: revive(publicKey(options)) })) as PublicKeyCredential | null
  if (!cred) throw new Error(t('core.webauthn.noPasskeyUsed'))
  const r = cred.response as AuthenticatorAssertionResponse
  return {
    id: cred.id,
    rawId: bytesToB64url(cred.rawId),
    type: cred.type,
    extensions: cred.getClientExtensionResults(),
    response: {
      authenticatorData: bytesToB64url(r.authenticatorData),
      clientDataJSON: bytesToB64url(r.clientDataJSON),
      signature: bytesToB64url(r.signature),
      userHandle: r.userHandle ? bytesToB64url(r.userHandle) : null,
    },
  }
}

/** What to tell the person when the browser's passkey prompt fails. */
export function passkeyErrorText(e: unknown): string {
  const name = e instanceof DOMException ? e.name : ''
  switch (name) {
    case 'NotAllowedError':
      return t('core.webauthn.passkeyPromptClosedTimed')
    case 'InvalidStateError':
      return t('core.webauthn.deviceAlreadyPasskeyHere')
    case 'SecurityError':
      return t('core.webauthn.browserWouldUsePasskey')
    case 'NotSupportedError':
      return t('core.webauthn.browserDeviceCannotMake')
    case 'AbortError':
      return t('core.webauthn.passkeyPromptCancelled')
    default:
      return e instanceof Error && e.message ? e.message : t('core.webauthn.passkeyDidWork')
  }
}
