import { register } from '../../index'

const messages = {
  'servers.serverControls.folderMissing': 'Folder missing',
  'servers.serverControls.startName': 'Start {name}',
  'servers.serverControls.playNodeStart': '{play, node} Start ',
  'servers.serverControls.stopName': 'Stop {name}',
  'servers.serverControls.squareNodeStop': '{square, node} Stop ',
  'servers.serverControls.restartName': 'Restart {name}',
  'servers.serverControls.rotatecwNodeRestart': '{rotateCw, node} Restart ',
  'servers.serverControls.restartName2': 'Restart {name}?',
  'servers.serverControls.everyonePlayingServerDisconnec': 'Everyone playing on this server will be disconnected for a minute or so while it restarts.',
  'servers.serverControls.restartServer': 'Restart server',
} as const

register('en', messages)
export default messages
