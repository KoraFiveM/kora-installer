import { register } from '../../index'
import type en from '../../en/servers/serversPage'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'servers.serversPage.addedFromExistingFolder': 'Aus einem vorhandenen Ordner hinzugefügt',
  'servers.serversPage.yourServers': 'Deine Server',
  'servers.serversPage.refreshcwNodeRefresh': '{refreshCw, node} Aktualisieren',
  'servers.serversPage.folderplusNodeAddExisting': '{folderPlus, node} Einen vorhandenen Server hinzufügen',
  'servers.serversPage.noServersYet': 'Noch keine Server',
  'servers.serversPage.koraSetUpNew': 'Kora kann in ein paar Minuten einen neuen FiveM-Server für dich einrichten: die Serverdateien, die Datenbank und die Resources. Oder füge einen hinzu, den du schon hast.',
  'servers.serversPage.plusNodeInstallFirst': '{plus, node} Ersten Server installieren',
  'servers.serversPage.healthSummary': 'Zustandsübersicht',
  'servers.serversPage.runningRunning': '{running} laufen',
  'servers.serversPage.stoppedStopped': '{stopped} angehalten',
  'servers.serversPage.noAlerts': 'Keine Warnungen',
  'servers.serversPage.gamePort': 'Spiel-Port',
  'servers.serversPage.lastBackup': 'Letztes Backup',
  'servers.serversPage.openCopy': 'Öffnen und kopieren',
  'servers.serversPage.alertsCount': '{count, plural, one {# Warnung} other {# Warnungen}}',
  'servers.serversPage.alertsNeedAttention': '{count, plural, one {# Warnung braucht Aufmerksamkeit} other {# Warnungen brauchen Aufmerksamkeit}}',
  'servers.serversPage.colName': 'Name',
  'servers.serversPage.colMode': 'Modus',
  'servers.serversPage.colStatus': 'Status',
  'servers.serversPage.colTxadminPort': 'txAdmin-Port',
  'servers.serversPage.colUpdates': 'Updates',
  'servers.serversPage.colActions': 'Aktionen',
} satisfies Record<keyof typeof en, string>

register('de', messages)
