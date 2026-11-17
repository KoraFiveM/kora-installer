import { register } from '../../index'
import type en from '../../en/servers/consoleTab'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'servers.consoleTab.consoleAvailable': 'Die Konsole steht dir nicht zur Verfügung',
  'servers.consoleTab.consoleSwitched': 'Die Konsole ist nicht eingeschaltet',
  'servers.consoleTab.showLiveServerOutput': 'Um die Live-Ausgabe des Servers zu zeigen und Befehle zu senden, fügt Kora diesem Server eine kleine Hilfs-Resource namens kora_console hinzu. Sie wird beim nächsten Start des Servers geladen.',
  'servers.consoleTab.enableConsole': 'Konsole aktivieren',
  'servers.consoleTab.serverRunningThereNo': 'Der Server läuft nicht, deshalb gibt es keine Live-Ausgabe.',
  'servers.consoleTab.startServer': 'Server starten',
  'servers.consoleTab.serverConsoleOutput': 'Ausgabe der Server-Konsole',
  'servers.consoleTab.waitingOutput': 'Warte auf Ausgabe...',
  'servers.consoleTab.arrowdownNodePausedJump': '{arrowDown, node} Angehalten. Zum Neuesten springen',
  'servers.consoleTab.consoleCommand': 'Konsolenbefehl',
  'servers.consoleTab.typeCommandExampleStatus': 'Gib einen Befehl ein, zum Beispiel status',
  'servers.consoleTab.send': 'Senden',
} satisfies Record<keyof typeof en, string>

register('de', messages)
