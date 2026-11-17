import { register } from '../../index'

const messages = {
  'servers.addExistingDialog.addExistingServer': 'Add an existing server',
  'servers.addExistingDialog.koraManageFivemServer': 'Kora can manage a FiveM server you set up another way. It will not change your files.',
  'servers.addExistingDialog.lookServersComputer': 'Look for servers on this computer',
  'servers.addExistingDialog.scanServers': 'Scan for servers',
  'servers.addExistingDialog.looking': 'Looking...',
  'servers.addExistingDialog.noNewServersWere': 'No new servers were found. You can type the folder below.',
  'servers.addExistingDialog.addName': 'Add {name}',
  'servers.addExistingDialog.add': 'Add',
  'servers.addExistingDialog.typeFolder': 'Or type the folder',
  'servers.addExistingDialog.serverFolder': 'Server folder',
  'servers.addExistingDialog.folderContainsServerCfg': 'The folder that contains server.cfg and resources, for example /srv/fivem/my-server.',
  'servers.addExistingDialog.srvFivemMyserver': '/srv/fivem/my-server',
  'servers.addExistingDialog.chooseServerFolder': 'Choose your server folder',
  'servers.addExistingDialog.browse': 'Browse...',
  'servers.addExistingDialog.addFolder': 'Add this folder',
  'servers.addExistingDialog.addedToList': '{name} was added to your list.',
} as const

register('en', messages)
export default messages
