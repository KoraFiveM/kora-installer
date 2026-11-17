import { register } from '../../index'

const messages = {
  'servers.portsTab.stopServerBeforeChanging': 'Stop the server before changing its ports.',
  'servers.portsTab.gamePort': 'Game port',
  'servers.portsTab.playersConnectPort': 'Players connect to this port.',
  'servers.portsTab.txadminPort': 'txAdmin port',
  'servers.portsTab.txadminWebPageUses': 'The txAdmin web page uses this port.',
  'servers.portsTab.changePorts': 'Change ports',
  'servers.portsTab.koraDoesChangeFirewall': 'Kora does not change your firewall. Open the new ports and close the old ones yourself.',
  'servers.portsTab.portsChanged': 'Ports changed',
  'servers.portsTab.ports': 'Ports',
} as const

register('en', messages)
export default messages
