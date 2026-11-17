import { register } from '../../index'

const messages = {
  'backups.backupSuggestion.dailyBackups': 'Daily backups are on',
  'backups.backupSuggestion.backupsTab': 'Backups tab',
  'backups.backupSuggestion.backupMadeEveryDay': "A backup is made every day at 03:30 while Kora's background service runs, and the newest 7 are kept. Change this on the <link>Backups tab</link>.",
  'backups.backupSuggestion.backServerUpEvery': 'Back this server up every day?',
  'backups.backupSuggestion.offUntilTurnChange': ' It is off until you turn it on, and you can change when, where to, and how many are kept.',
  'backups.backupSuggestion.diskFailsUpdateGoes': "A disk that fails or an update that goes wrong can take your players' data with it. Kora can make a backup of the database, settings and resources every day.{value}",
  'backups.backupSuggestion.turnDailyBackups': 'Turn on daily backups',
  'backups.backupSuggestion.backupSettings': 'Backup settings',
} as const

register('en', messages)
export default messages
