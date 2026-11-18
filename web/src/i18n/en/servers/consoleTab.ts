import { register } from '../../index'

const messages = {
  'servers.consoleTab.consoleAvailable': 'The console is not available to you',
  'servers.consoleTab.consoleSwitched': 'The console is not switched on',
  'servers.consoleTab.showLiveServerOutput': 'To show live server output and send commands, Kora adds a small helper resource called kora_console to this server. It loads the next time the server starts.',
  'servers.consoleTab.enableConsole': 'Enable console',
  'servers.consoleTab.serverRunningThereNo': 'The server is not running, so there is no live output.',
  'servers.consoleTab.startServer': 'Start server',
  'servers.consoleTab.serverConsoleOutput': 'Server console output',
  'servers.consoleTab.waitingOutput': 'Waiting for output...',
  'servers.consoleTab.arrowdownNodePausedJump': '{arrowDown, node} Paused. Jump to latest',
  'servers.consoleTab.consoleCommand': 'Console command',
  'servers.consoleTab.typeCommandExampleStatus': 'Type a command, for example status',
  'servers.consoleTab.send': 'Send',
} as const

register('en', messages)
export default messages
