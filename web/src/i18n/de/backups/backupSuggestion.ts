import { register } from '../../index'
import type en from '../../en/backups/backupSuggestion'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'backups.backupSuggestion.dailyBackups': 'Tägliche Backups sind an',
  'backups.backupSuggestion.backupMadeEveryDay': 'Jeden Tag um 03:30 wird ein Backup erstellt, solange Koras Hintergrunddienst läuft, und die neuesten 7 bleiben erhalten. Ändere das im <link>Tab Backups</link>.',
  'backups.backupSuggestion.backServerUpEvery': 'Diesen Server jeden Tag sichern?',
  'backups.backupSuggestion.offUntilTurnChange': ' Es ist aus, bis du es einschaltest, und du kannst ändern, wann, wohin und wie viele aufbewahrt werden.',
  'backups.backupSuggestion.diskFailsUpdateGoes': 'Ein Festplattenausfall oder ein missglücktes Update kann die Daten deiner Spieler mitreißen. Kora kann jeden Tag ein Backup von Datenbank, Einstellungen und Resources erstellen.{value}',
  'backups.backupSuggestion.turnDailyBackups': 'Tägliche Backups einschalten',
  'backups.backupSuggestion.backupSettings': 'Backup-Einstellungen',
} satisfies Record<keyof typeof en, string>

register('de', messages)
