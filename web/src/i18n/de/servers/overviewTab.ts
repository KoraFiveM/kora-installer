import { register } from '../../index'
import type en from '../../en/servers/overviewTab'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'servers.overviewTab.status': 'Status',
  'servers.overviewTab.server': 'Server',
  'servers.overviewTab.folderMissing': 'Ordner fehlt',
  'servers.overviewTab.gamePortGame': 'Spiel-Port {game}',
  'servers.overviewTab.notListening': 'Hört nicht zu',
  'servers.overviewTab.txadminPortTxadmin': 'txAdmin-Port {txadmin}',
  'servers.overviewTab.notAnswering': 'Antwortet nicht',
  'servers.overviewTab.updates': 'Updates',
  'servers.overviewTab.backups': 'Backups',
  'servers.overviewTab.serverFolderCannotFound': 'Der Serverordner wurde nicht gefunden. Er wurde vielleicht verschoben oder gelöscht. Du kannst diesen Eintrag im Tab Gefahrenzone entfernen.',
  'servers.overviewTab.connectManage': 'Verbinden und verwalten',
  'servers.overviewTab.txadminAnswersOnceServer': 'txAdmin antwortet, sobald der Server läuft.',
  'servers.overviewTab.connectAddressPlayersType': 'Verbindungsadresse (Spieler geben sie in der FiveM-Konsole ein oder nutzen Direktverbindung)',
  'servers.overviewTab.connectAddress': 'Verbindungsadresse',
  'servers.overviewTab.koraCouldFindComputers': 'Kora konnte die Netzwerkadresse dieses Computers nicht finden. Spieler verbinden sich mit der Adresse dieses Computers und dem Port {game}.',
  'servers.overviewTab.serverFolder': 'Serverordner',
  'servers.overviewTab.openingFolderOnlyWorks': 'Das Öffnen des Ordners funktioniert nur auf dem Computer, auf dem Kora läuft, an dessen Bildschirm. Kopiere von hier aus den Pfad.',
  'servers.overviewTab.runs': 'Läuft in',
  'servers.overviewTab.inAContainer': 'In einem Container',
  'servers.overviewTab.running': 'Läuft',
  'servers.overviewTab.stopped': 'Angehalten',
  'servers.overviewTab.listening': 'Hört zu',
  'servers.overviewTab.answering': 'Antwortet',
} satisfies Record<keyof typeof en, string>

register('de', messages)
