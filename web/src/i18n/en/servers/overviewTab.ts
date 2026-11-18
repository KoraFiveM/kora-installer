import { register } from '../../index'

const messages = {
  'servers.overviewTab.status': 'Status',
  'servers.overviewTab.server': 'Server',
  'servers.overviewTab.folderMissing': 'Folder missing',
  'servers.overviewTab.gamePortGame': 'Game port {game}',
  'servers.overviewTab.notListening': 'Not listening',
  'servers.overviewTab.txadminPortTxadmin': 'txAdmin port {txadmin}',
  'servers.overviewTab.notAnswering': 'Not answering',
  'servers.overviewTab.updates': 'Updates',
  'servers.overviewTab.backups': 'Backups',
  'servers.overviewTab.serverFolderCannotFound': 'The server folder cannot be found. It may have been moved or deleted. You can remove this entry on the Danger zone tab.',
  'servers.overviewTab.connectManage': 'Connect and manage',
  'servers.overviewTab.txadminAnswersOnceServer': 'txAdmin answers once the server is running.',
  'servers.overviewTab.connectAddressPlayersType': 'Connect address (players type this in the FiveM console, or use direct connect)',
  'servers.overviewTab.connectAddress': 'Connect address',
  'servers.overviewTab.koraCouldFindComputers': "Kora could not find this computer's network address. Players connect to this computer's address with port {game}.",
  'servers.overviewTab.serverFolder': 'Server folder',
  'servers.overviewTab.openingFolderOnlyWorks': 'Opening the folder only works on the computer that runs Kora, at its screen. From here, copy the path.',
  'servers.overviewTab.runs': 'Runs',
  'servers.overviewTab.inAContainer': 'In a container',
  'servers.overviewTab.running': 'Running',
  'servers.overviewTab.stopped': 'Stopped',
  'servers.overviewTab.listening': 'Listening',
  'servers.overviewTab.answering': 'Answering',
} as const

register('en', messages)
export default messages
