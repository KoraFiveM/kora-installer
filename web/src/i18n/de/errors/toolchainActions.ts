import { register } from '../../index'
import type en from '../../en/errors/toolchainActions'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'errors.toolchainActions.settingUpToolName': '{tool, select, git {Git} gh {Die GitHub-CLI} other {{tool}}} wird eingerichtet',
  'errors.toolchainActions.startingDatabaseServer': 'Der Datenbankserver wird gestartet',
  'errors.toolchainActions.databaseServerRunning': 'Der Datenbankserver läuft.',
  'errors.toolchainActions.signGithub': 'Bei GitHub anmelden',
  'errors.toolchainActions.startDatabase': 'Die Datenbank starten',
  'errors.toolchainActions.koraKeepsItsOwn': 'Kora bewahrt seine eigene Kopie in einem eigenen Ordner auf. Sie wird vorher gegen eine bekannte Prüfsumme geprüft, braucht keine Administratorrechte und ändert sonst nichts auf diesem Computer.',
  'errors.toolchainActions.databaseServerInstalledBut': 'Der Datenbankserver ist installiert, läuft aber nicht. Zum Starten sind eventuell Administratorrechte nötig.',
  'errors.toolchainActions.close': 'Schließen',
  'errors.toolchainActions.done': 'Erledigt',
  'errors.toolchainActions.carryingWhatWereDoing': ' Es geht mit dem weiter, was du gerade getan hast.',
  'errors.toolchainActions.somethingWaitingTryAgain': ' Wenn etwas darauf gewartet hat, kannst du es jetzt erneut versuchen.',
  'errors.toolchainActions.keepPageOpenCarries': 'Du kannst diese Seite offen lassen; es läuft von selbst weiter.',
  'errors.toolchainActions.checkcircleNodeSignedValue': '{checkCircle2, node}Du bist angemeldet.{value}',
  'errors.toolchainActions.installToolName': '{tool, select, git {Git} gh {Die GitHub-CLI} other {{tool}}} installieren',
  'errors.toolchainActions.toolReady': '{tool, select, git {Git ist bereit.} gh {Die GitHub-CLI ist bereit.} other {{tool} ist bereit.}}',
} satisfies Record<keyof typeof en, string>

register('de', messages)
