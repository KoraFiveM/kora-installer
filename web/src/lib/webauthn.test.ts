import { describe, expect, it } from 'vitest'
import { b64urlToBytes, bytesToB64url, passkeyErrorText } from './webauthn'

describe('base64url', () => {
  it('round-trips every byte value, without padding or + and /', () => {
    const all = new Uint8Array(256).map((_, i) => i)
    const s = bytesToB64url(all)
    expect(s).not.toMatch(/[+/=]/)
    expect(Array.from(b64urlToBytes(s))).toEqual(Array.from(all))
  })
  it('reads the examples of RFC 4648 (url-safe)', () => {
    expect(bytesToB64url(new TextEncoder().encode('foobar'))).toBe('Zm9vYmFy')
    expect(new TextDecoder().decode(b64urlToBytes('Zm9vYg'))).toBe('foob')
    expect(Array.from(b64urlToBytes('-_8'))).toEqual([251, 255])
  })
})

describe('passkey errors', () => {
  it('say what to do', () => {
    expect(passkeyErrorText(new DOMException('x', 'NotAllowedError'))).toMatch(/closed, timed out or refused/)
    expect(passkeyErrorText(new DOMException('x', 'SecurityError'))).toMatch(/one-time code/)
    expect(passkeyErrorText(new DOMException('x', 'InvalidStateError'))).toMatch(/already has a passkey/)
    expect(passkeyErrorText(new Error('boom'))).toBe('boom')
    expect(passkeyErrorText('x')).toBe('The passkey did not work.')
  })
})
