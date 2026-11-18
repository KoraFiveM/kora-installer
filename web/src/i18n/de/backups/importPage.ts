import { register } from '../../index'
import type en from '../../en/backups/importPage'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'backups.importPage.bringServerFromBackup': 'Einen Server aus einem Backup oder von einem anderen Rechner holen',
  'backups.importPage.accountRoleCannotBring': 'Dein Konto ({role}) kann keinen Server hereinholen. Frag einen Administrator.',
  'backups.importPage.backServers': 'Zurück zu deinen Servern',
  'backups.importPage.restoreServerFromBackup': 'Stelle einen Server aus einer Backup-Datei wieder her oder hole einen von einem anderen Computer hierher (Linux oder Windows). Öffne auf dem alten Computer den Tab Backups des Servers und wähle „Diesen Server umziehen“, um die Datei zu erstellen. Außer der Datei, die du selbst überbringst, verlässt nichts deine Computer.',
  'backups.importPage.theFile': '1. Die Datei',
  'backups.importPage.thePassword': '2. Das Passwort',
  'backups.importPage.whatHappen': '3. Was passieren wird',
  'backups.importPage.bringingItIn': '4. Wird hereingeholt',
  'backups.importPage.done': 'Fertig',
  'backups.importPage.theFile2': 'Die Datei',
} satisfies Record<keyof typeof en, string>

register('de', messages)
