import { register } from '../../index'

const messages = {
  'resources.resourceActions.serverRunningStartServer': 'The server is not running. Start the server first; its resources start with it.',
  'resources.resourceActions.koraSendsTheseCommands': 'Kora sends these commands through the live console, and it is not switched on yet. Turn it on in the Console tab (it needs a restart).',
  'resources.resourceActions.liveConsoleSwitchedBut': 'The live console is switched on but the server has not been restarted since. Restart the server once and these buttons work.',
  'resources.resourceActions.updates': 'Updates',
  'resources.resourceActions.nodeNodeCheckUpdate': '{node, node} Check for update ',
  'resources.resourceActions.checkUpdateFirst': 'Check for an update first',
  'resources.resourceActions.nodeNodeUpdate': '{node, node} Update ',
  'resources.resourceActions.checking': 'Checking...',
  'resources.resourceActions.lastCheckedTimeago': 'Last checked {timeAgo}',
  'resources.resourceActions.behindNewerChangeValue': '{behind, plural, one {# newer change available.} other {# newer changes available.}}',
  'resources.resourceActions.updatedRunningServerStill': 'Updated. The running server still has the old version until it is restarted.',
  'resources.resourceActions.runningServer': 'Running on the server',
  'resources.resourceActions.runningServer2': 'On the running server',
  'resources.resourceActions.koraUsesResourceSend': 'Kora uses this resource to send commands to the server, so it cannot be stopped from here.',
  'resources.resourceActions.startServer': 'Start the server',
  'resources.resourceActions.openConsoleTab': 'Open the Console tab',
  'resources.resourceActions.behindNewerChangeWith': '{behind, plural, one {# newer change available: {changes}.} other {# newer changes available: {changes}.}}',
  'resources.resourceActions.start': 'Start',
  'resources.resourceActions.stop': 'Stop',
  'resources.resourceActions.restart': 'Restart',
} as const

register('en', messages)
export default messages
