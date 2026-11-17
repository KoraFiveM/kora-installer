import { register } from '../../index'
import type en from '../../en/servers/serverDetailPage'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'servers.serverDetailPage.arrowleftNodeAllServers': '{arrowLeft, node} Alle Server',
  'servers.serverDetailPage.serverSections': 'Server-Bereiche',
  'servers.serverDetailPage.overview': 'Übersicht',
  'servers.serverDetailPage.console': 'Konsole',
  'servers.serverDetailPage.logs': 'Logs',
  'servers.serverDetailPage.monitoring': 'Überwachung',
  'servers.serverDetailPage.resources': 'Resources',
  'servers.serverDetailPage.paidAssets': 'Bezahlte Assets',
  'servers.serverDetailPage.backups': 'Backups',
  'servers.serverDetailPage.schedule': 'Zeitplan',
  'servers.serverDetailPage.notifications': 'Benachrichtigungen',
  'servers.serverDetailPage.ports': 'Ports',
  'servers.serverDetailPage.dangerZone': 'Gefahrenzone',
} satisfies Record<keyof typeof en, string>

register('de', messages)
