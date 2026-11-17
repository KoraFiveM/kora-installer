import { register } from '../../index'

const messages = {
  'servers.serverDetailPage.arrowleftNodeAllServers': '{arrowLeft, node} All servers',
  'servers.serverDetailPage.serverSections': 'Server sections',
  'servers.serverDetailPage.overview': 'Overview',
  'servers.serverDetailPage.console': 'Console',
  'servers.serverDetailPage.logs': 'Logs',
  'servers.serverDetailPage.monitoring': 'Monitoring',
  'servers.serverDetailPage.resources': 'Resources',
  'servers.serverDetailPage.paidAssets': 'Paid assets',
  'servers.serverDetailPage.backups': 'Backups',
  'servers.serverDetailPage.schedule': 'Schedule',
  'servers.serverDetailPage.notifications': 'Notifications',
  'servers.serverDetailPage.ports': 'Ports',
  'servers.serverDetailPage.dangerZone': 'Danger zone',
} as const

register('en', messages)
export default messages
