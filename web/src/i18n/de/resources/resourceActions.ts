import { register } from '../../index'
import type en from '../../en/resources/resourceActions'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'resources.resourceActions.serverRunningStartServer': 'Der Server läuft nicht. Starte zuerst den Server; seine Resources starten mit ihm.',
  'resources.resourceActions.koraSendsTheseCommands': 'Kora sendet diese Befehle über die Live-Konsole, und sie ist noch nicht eingeschaltet. Schalte sie im Tab Konsole ein (sie braucht einen Neustart).',
  'resources.resourceActions.liveConsoleSwitchedBut': 'Die Live-Konsole ist eingeschaltet, aber der Server wurde seitdem nicht neu gestartet. Starte den Server einmal neu, dann funktionieren diese Knöpfe.',
  'resources.resourceActions.updates': 'Updates',
  'resources.resourceActions.nodeNodeCheckUpdate': '{node, node} Auf Update prüfen ',
  'resources.resourceActions.checkUpdateFirst': 'Prüfe zuerst auf ein Update',
  'resources.resourceActions.nodeNodeUpdate': '{node, node} Aktualisieren ',
  'resources.resourceActions.checking': 'Wird geprüft...',
  'resources.resourceActions.lastCheckedTimeago': 'Zuletzt geprüft {timeAgo}',
  'resources.resourceActions.behindNewerChangeValue': '{behind, plural, one {# neuere Änderung verfügbar.} other {# neuere Änderungen verfügbar.}}',
  'resources.resourceActions.updatedRunningServerStill': 'Aktualisiert. Der laufende Server hat bis zu seinem Neustart noch die alte Version.',
  'resources.resourceActions.runningServer': 'Auf dem Server aktiv',
  'resources.resourceActions.runningServer2': 'Auf dem laufenden Server',
  'resources.resourceActions.koraUsesResourceSend': 'Kora nutzt diese Resource, um Befehle an den Server zu senden, deshalb kann sie von hier nicht gestoppt werden.',
  'resources.resourceActions.startServer': 'Server starten',
  'resources.resourceActions.openConsoleTab': 'Tab Konsole öffnen',
  'resources.resourceActions.behindNewerChangeWith': '{behind, plural, one {# neuere Änderung verfügbar: {changes}.} other {# neuere Änderungen verfügbar: {changes}.}}',
  'resources.resourceActions.start': 'Starten',
  'resources.resourceActions.stop': 'Stoppen',
  'resources.resourceActions.restart': 'Neu starten',
} satisfies Record<keyof typeof en, string>

register('de', messages)
