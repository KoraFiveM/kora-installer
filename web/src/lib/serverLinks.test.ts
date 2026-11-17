import { describe, expect, it } from 'vitest'
import { isLoopbackHost, txAdminLink } from './serverLinks'

describe('txAdminLink', () => {
  it('uses localhost when the browser is on the computer that runs Kora', () => {
    expect(txAdminLink('localhost', 40120, true)).toEqual({ url: 'http://localhost:40120', hint: null })
    expect(txAdminLink('127.0.0.1', 40121, true).url).toBe('http://localhost:40121')
  })

  it('uses the name the panel was reached by when that is another computer (Tailscale, LAN)', () => {
    const l = txAdminLink('my-server.tail1234.ts.net', 40120, false)
    expect(l.url).toBe('http://my-server.tail1234.ts.net:40120')
    expect(l.hint).toContain('ssh -L 40120:localhost:40120')
    expect(txAdminLink('100.64.0.5', 40125, false).url).toBe('http://100.64.0.5:40125')
  })

  it('tells someone on an SSH tunnel that txAdmin needs its own tunnel', () => {
    const l = txAdminLink('localhost', 40120, false)
    expect(l.url).toBe('http://localhost:40120')
    expect(l.hint).toContain('-L 40120:localhost:40120')
  })

  it('knows loopback names', () => {
    for (const h of ['localhost', 'LOCALHOST', '127.0.0.1', '127.1.2.3', '[::1]']) expect(isLoopbackHost(h), h).toBe(true)
    for (const h of ['100.64.0.5', 'example.com', '192.168.1.5', '[fd7a::1]']) expect(isLoopbackHost(h), h).toBe(false)
  })
})
