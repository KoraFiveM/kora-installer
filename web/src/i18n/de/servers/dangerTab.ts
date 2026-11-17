import { register } from '../../index'
import type en from '../../en/servers/dangerTab'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'servers.dangerTab.removeFromList': 'Aus der Liste entfernen',
  'servers.dangerTab.addedServerFromExisting': 'Du hast diesen Server aus einem vorhandenen Ordner hinzugefügt. Kora verwaltet ihn dann nicht mehr, aber deine Dateien und deine Datenbank werden nicht angefasst.',
  'servers.dangerTab.removeNameFromList': '{name} aus der Liste entfernen?',
  'servers.dangerTab.filesStayWhereThey': 'Deine Dateien bleiben, wo sie sind.',
  'servers.dangerTab.deleteServer': 'Server löschen',
  'servers.dangerTab.stopsServerRemovesIts': 'Hält den Server an, entfernt seinen Hintergrunddienst und seine Firewall-Regeln und nimmt ihn aus deiner Liste. Du entscheidest, was mit den Dateien und der Datenbank passiert.',
  'servers.dangerTab.deleteServer2': 'Server löschen...',
  'servers.dangerTab.serverCouldDeleted': 'Der Server konnte nicht gelöscht werden',
  'servers.dangerTab.databaseLoginNeeded': 'Datenbank-Anmeldung nötig',
  'servers.dangerTab.databaseAdministratorPassword': 'Passwort des Datenbank-Administrators',
  'servers.dangerTab.usedOnceDeleteDatabase': 'Wird einmal zum Löschen der Datenbank verwendet. Kora speichert es nicht.',
  'servers.dangerTab.tryAgainPassword': 'Mit diesem Passwort erneut versuchen',
  'servers.dangerTab.keepDatabaseFinish': 'Die Datenbank behalten und abschließen',
  'servers.dangerTab.keepServerFiles': 'Die Serverdateien behalten?',
  'servers.dangerTab.keepDatabase': 'Die Datenbank behalten?',
  'servers.dangerTab.deleteName': '{name} löschen',
  'servers.dangerTab.folderDir': 'Der Ordner ist {dir}',
  'servers.dangerTab.databaseHoldsPlayersCharacters': 'Die Datenbank enthält deine Spieler, Charaktere und alles, was ihnen gehört.',
  'servers.dangerTab.cannotUndone': 'Das lässt sich nicht rückgängig machen.',
  'servers.dangerTab.next': 'Weiter',
  'servers.dangerTab.serverFiles': 'Serverdateien',
  'servers.dangerTab.inputNodeKeepFiles': '{input, node} Die Dateien behalten',
  'servers.dangerTab.inputNodeDeleteFiles': '{input, node} Die Dateien löschen',
  'servers.dangerTab.database': 'Datenbank',
  'servers.dangerTab.inputNodeKeepDatabase': '{input, node} Die Datenbank behalten',
  'servers.dangerTab.inputNodeDeleteDatabase': '{input, node} Die Datenbank löschen',
  'servers.dangerTab.filesKept': 'Dateien bleiben erhalten.',
  'servers.dangerTab.filesDeleted': 'Die Dateien werden gelöscht.',
  'servers.dangerTab.databaseKept': 'Datenbank bleibt erhalten.',
  'servers.dangerTab.databaseDeleted': 'Die Datenbank wird gelöscht.',
  'servers.dangerTab.typeNameConfirm': 'Tippe {name} zur Bestätigung',
  'servers.dangerTab.deleted': '{name} wurde gelöscht.',
  'servers.dangerTab.removedFromList': '{name} wurde aus deiner Liste entfernt.',
} satisfies Record<keyof typeof en, string>

register('de', messages)
