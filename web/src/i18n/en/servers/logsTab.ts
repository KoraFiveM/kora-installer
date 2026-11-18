import { register } from '../../index'

const messages = {
  'servers.logsTab.logsAvailable': 'The logs are not available to you',
  'servers.logsTab.logFile': 'Log file',
  'servers.logsTab.refreshcwNodeReload': '{refreshCw, node} Reload',
  'servers.logsTab.copyLog': 'Copy log',
  'servers.logsTab.thereNoLogFiles': 'There are no log files yet. They appear after the server has run.',
  'servers.logsTab.lastLinesFile': 'Last lines of {file}',
  'servers.logsTab.fileEmpty': 'This file is empty.',
  'servers.logsTab.loading': 'Loading...',
  'servers.logsTab.showsLastLines': 'Shows the last 300 lines.',
} as const

register('en', messages)
export default messages
