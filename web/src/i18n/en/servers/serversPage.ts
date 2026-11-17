import { register } from '../../index'

const messages = {
  'servers.serversPage.addedFromExistingFolder': 'Added from an existing folder',
  'servers.serversPage.yourServers': 'Your servers',
  'servers.serversPage.refreshcwNodeRefresh': '{refreshCw, node} Refresh',
  'servers.serversPage.folderplusNodeAddExisting': '{folderPlus, node} Add an existing server',
  'servers.serversPage.noServersYet': 'No servers yet',
  'servers.serversPage.koraSetUpNew': 'Kora can set up a new FiveM server for you in a few minutes: the server files, database and resources. Or add one you already have.',
  'servers.serversPage.plusNodeInstallFirst': '{plus, node} Install your first server',
  'servers.serversPage.healthSummary': 'Health summary',
  'servers.serversPage.runningRunning': '{running} running',
  'servers.serversPage.stoppedStopped': '{stopped} stopped',
  'servers.serversPage.noAlerts': 'No alerts',
  'servers.serversPage.gamePort': 'Game port',
  'servers.serversPage.lastBackup': 'Last backup',
  'servers.serversPage.openCopy': 'Open and copy',
  'servers.serversPage.alertsCount': '{count, plural, one {# alert} other {# alerts}}',
  'servers.serversPage.alertsNeedAttention': '{count, plural, one {# alert needs attention} other {# alerts need attention}}',
} as const

register('en', messages)
export default messages
