import { register } from '../../index'

const messages = {
  'core.backupChip.textNeedsLook': '{text} (needs a look)',
  'core.backupChip.lastBackup': 'Last backup {ago}',
  'core.backupChip.noBackupYet': 'No backup yet',
} as const

register('en', messages)
export default messages
