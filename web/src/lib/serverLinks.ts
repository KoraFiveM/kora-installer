import { t, msg } from '@/i18n'
import '@/i18n/en/core/serverLinks'
// Where the per-server buttons point, worked out from how the person reached the panel.

/** Is this the name of the computer the browser is on (so `localhost` means the same machine as Kora)? */
export function isLoopbackHost(host: string): boolean {
  const h = host.toLowerCase()
  return h === 'localhost' || h === '[::1]' || h === '::1' || h.startsWith('127.')
}

/**
 * The panel-side port of a server's txAdmin: ten thousand above txAdmin's own port (40120 -> 50120), or ten thousand
 * below when that would leave the port range. The same fixed rule as the panel's (`txgate::gateway_port`), so the hint
 * can name the port before anything is opened.
 */
export function gatewayPort(txadmin: number): number | null {
  const up = txadmin + 10000
  const p = up <= 65535 ? up : txadmin - 10000
  return p >= 1024 ? p : null
}

export interface TxAdminLink {
  /** A direct address, or null when txAdmin opens through the panel (a one-time ticket, see `viaPanel`). */
  url: string | null
  /** Open it through the panel: the panel signs the person in to its own gateway first. */
  viaPanel: boolean
  /** Plain words about what the button does or how to get through, or null when there is nothing to add. */
  hint: string | null
}

/**
 * How a server's txAdmin is reached from THIS browser.
 *
 * - Browser on the same computer as Kora (`local`): `localhost` and txAdmin's own port, as before.
 * - Anywhere else (another device by name or address, Tailscale, an SSH tunnel to a machine with no desktop): through the
 *   panel. txAdmin cannot be served under a path of the panel (it uses absolute addresses), so the panel opens one more
 *   port for it on the same address it is already reached at, signs the person in with their panel session and passes
 *   everything on to txAdmin. txAdmin's own port does not need to be open for this.
 */
export function txAdminLink(host: string, port: number, local: boolean): TxAdminLink {
  if (isLoopbackHost(host) && local) return { url: `http://localhost:${port}`, viaPanel: false, hint: null }
  const gw = gatewayPort(port)
  if (isLoopbackHost(host)) {
    return {
      url: null,
      viaPanel: true,
      hint: gw === null ? t('core.serverLinks.reachedPanelThroughTunnelOwn') : t('core.serverLinks.reachedPanelThroughTunnel', { gw }),
    }
  }
  return {
    url: null,
    viaPanel: true,
    hint: gw === null ? t('core.serverLinks.opensTxadminThroughPanelOwn', { host }) : t('core.serverLinks.opensTxadminThroughPanel', { gw, host }),
  }
}

/** What to say next to the connect address. The address is the network one: players elsewhere need more. */
export const CONNECT_NOTE =
  msg('core.serverLinks.addressOwnNetworkPlayers')
