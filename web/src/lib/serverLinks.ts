// Where the per-server buttons point, worked out from how the person reached the panel.

/** Is this the name of the computer the browser is on (so `localhost` means the same machine as Kora)? */
export function isLoopbackHost(host: string): boolean {
  const h = host.toLowerCase()
  return h === 'localhost' || h === '[::1]' || h === '::1' || h.startsWith('127.')
}

export interface TxAdminLink {
  url: string
  /** Plain words about when the address will not work and what to do, or null when there is nothing to add. */
  hint: string | null
}

/**
 * The address of a server's txAdmin for THIS browser.
 *
 * - Browser on the same computer as Kora (`local`): `localhost`, which is right.
 * - Reached by name or address (LAN, Tailscale): the same name with txAdmin's port. txAdmin's port is closed to other
 *   computers unless it was opened on purpose, so the hint says how to get through if the page does not load.
 * - Reached through an SSH tunnel (`localhost` but no desktop in front of the server): `localhost` is the person's
 *   own computer, so txAdmin needs its own tunnel. The hint gives the command.
 */
export function txAdminLink(host: string, port: number, local: boolean): TxAdminLink {
  if (!isLoopbackHost(host)) {
    return {
      url: `http://${host}:${port}`,
      hint: `This opens ${host} on port ${port}. If the page does not load, that port is closed to other computers (the safe default). Open it with an SSH tunnel instead: ssh -L ${port}:localhost:${port} <user>@${host}, then use http://localhost:${port}.`,
    }
  }
  if (local) return { url: `http://localhost:${port}`, hint: null }
  return {
    url: `http://localhost:${port}`,
    hint: `You reached this panel through a tunnel, so this only works if txAdmin's port is tunnelled too. Add it to your ssh command: -L ${port}:localhost:${port}`,
  }
}

/** What to say next to the connect address. The address is the network one: players elsewhere need more. */
export const CONNECT_NOTE =
  'This is the address on your own network: players on the same network (or Wi-Fi) can use it. Players on the internet need your public address with this port, and the port has to be forwarded on your router (or opened with your hosting provider), because the server sits behind it. Kora cannot see your public address: search "what is my IP" from this computer, or use your domain name.'
