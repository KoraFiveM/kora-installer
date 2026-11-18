import { register } from '../../index'
import type en from '../../en/servers/logsTab'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'servers.logsTab.logsAvailable': 'Die Logs stehen dir nicht zur Verfügung',
  'servers.logsTab.logFile': 'Log-Datei',
  'servers.logsTab.refreshcwNodeReload': '{refreshCw, node} Neu laden',
  'servers.logsTab.copyLog': 'Log kopieren',
  'servers.logsTab.thereNoLogFiles': 'Es gibt noch keine Log-Dateien. Sie erscheinen, nachdem der Server gelaufen ist.',
  'servers.logsTab.lastLinesFile': 'Letzte Zeilen von {file}',
  'servers.logsTab.fileEmpty': 'Diese Datei ist leer.',
  'servers.logsTab.loading': 'Lädt...',
  'servers.logsTab.showsLastLines': 'Zeigt die letzten 300 Zeilen.',
} satisfies Record<keyof typeof en, string>

register('de', messages)
