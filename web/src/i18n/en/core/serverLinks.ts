import { register } from '../../index'

const messages = {
  'core.serverLinks.reachedPanelThroughTunnel': 'You reached this panel through a tunnel. txAdmin opens through the panel on port {gw}, so add -L {gw}:localhost:{gw} to your ssh command, then press the button.',
  'core.serverLinks.opensTxadminThroughPanel': 'Opens txAdmin through this panel (you stay signed in with your panel sign-in), on port {gw} of {host}. If the page does not load, a firewall is blocking that port: allow TCP port {gw} for the devices that use the panel.',
  'core.serverLinks.reachedPanelThroughTunnelOwn': 'You reached this panel through a tunnel. txAdmin opens through the panel on a port of its own, so add a -L option for that port to your ssh command, then press the button.',
  'core.serverLinks.opensTxadminThroughPanelOwn': 'Opens txAdmin through this panel (you stay signed in with your panel sign-in), on a port of its own on {host}. If the page does not load, a firewall is blocking that port: allow it for the devices that use the panel.',
  'core.serverLinks.addressOwnNetworkPlayers': 'This is the address on your own network: players on the same network (or Wi-Fi) can use it. Players on the internet need your public address with this port, and the port has to be open on your firewall and forwarded on your router (or opened with your hosting provider). The check below ("Can players reach this server?") finds your public address, looks at each of those steps and can set up the port forwarding for you.',
} as const

register('en', messages)
export default messages
