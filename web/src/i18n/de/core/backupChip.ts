import { register } from '../../index'
import type en from '../../en/core/backupChip'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.backupChip.textNeedsLook': '{text} (braucht einen Blick)',
  'core.backupChip.lastBackup': 'Letztes Backup {ago}',
  'core.backupChip.noBackupYet': 'Noch kein Backup',
} satisfies Record<keyof typeof en, string>

register('de', messages)
