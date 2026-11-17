import { register } from '../../index'
import type en from '../../en/servers/portsTab'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'servers.portsTab.stopServerBeforeChanging': 'Halte den Server an, bevor du seine Ports änderst.',
  'servers.portsTab.gamePort': 'Spiel-Port',
  'servers.portsTab.playersConnectPort': 'Spieler verbinden sich mit diesem Port.',
  'servers.portsTab.txadminPort': 'txAdmin-Port',
  'servers.portsTab.txadminWebPageUses': 'Die txAdmin-Webseite nutzt diesen Port.',
  'servers.portsTab.changePorts': 'Ports ändern',
  'servers.portsTab.koraDoesChangeFirewall': 'Kora ändert deine Firewall nicht. Öffne die neuen Ports und schließe die alten selbst.',
  'servers.portsTab.portsChanged': 'Ports geändert',
  'servers.portsTab.ports': 'Ports',
} satisfies Record<keyof typeof en, string>

register('de', messages)
