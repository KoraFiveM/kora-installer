import { describe, expect, it } from 'vitest'
import { gatewayPort, isLoopbackHost, txAdminLink } from './serverLinks'

describe('txAdminLink', () => {
  it('uses localhost and txAdmin\'s own port when the browser is on the computer that runs Kora', () => {
    expect(txAdminLink('localhost', 40120, true)).toEqual({ url: 'http://localhost:40120', viaPanel: false, hint: null })
    expect(txAdminLink('127.0.0.1', 40121, true).url).toBe('http://localhost:40121')
  })

  it('goes through the panel from another computer (LAN, Tailscale), naming the port to allow', () => {
    const l = txAdminLink('my-server.tail1234.ts.net', 40120, false)
    expect(l.url).toBeNull()
    expect(l.viaPanel).toBe(true)
    expect(l.hint).toContain('port 50120')
    expect(l.hint).toContain('allow TCP port 50120')
    expect(txAdminLink('100.64.0.5', 40125, false).hint).toContain('port 50125')
  })

  it('tells someone on an SSH tunnel to tunnel the panel-side port, not txAdmin\'s', () => {
    const l = txAdminLink('localhost', 40120, false)
    expect(l.viaPanel).toBe(true)
    expect(l.hint).toContain('-L 50120:localhost:50120')
  })

  it('gives each txAdmin port a panel-side port by a fixed rule', () => {
    expect(gatewayPort(40120)).toBe(50120)
    expect(gatewayPort(55535)).toBe(65535)
    expect(gatewayPort(55536)).toBe(45536)
    expect(gatewayPort(65535)).toBe(55535)
  })

  it('knows loopback names', () => {
    for (const h of ['localhost', 'LOCALHOST', '127.0.0.1', '127.1.2.3', '[::1]']) expect(isLoopbackHost(h), h).toBe(true)
    for (const h of ['100.64.0.5', 'example.com', '192.168.1.5', '[fd7a::1]']) expect(isLoopbackHost(h), h).toBe(false)
  })
})
