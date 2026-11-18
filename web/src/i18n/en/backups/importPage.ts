import { register } from '../../index'

const messages = {
  'backups.importPage.bringServerFromBackup': 'Bring a server from a backup or another machine',
  'backups.importPage.accountRoleCannotBring': 'Your account ({role}) cannot bring a server in. Ask an administrator.',
  'backups.importPage.backServers': 'Back to your servers',
  'backups.importPage.restoreServerFromBackup': 'Restore a server from a backup file, or move one here from another computer (Linux or Windows). On the old computer open the server\'s Backups tab and choose "Move this server" to make the file. Nothing leaves your computers except the file you carry yourself.',
  'backups.importPage.theFile': '1. The file',
  'backups.importPage.thePassword': '2. The password',
  'backups.importPage.whatHappen': '3. What will happen',
  'backups.importPage.bringingItIn': '4. Bringing it in',
  'backups.importPage.done': 'Done',
  'backups.importPage.theFile2': 'The file',
} as const

register('en', messages)
export default messages
