import { register } from '../../index'
import type en from '../../en/settings/auditLog'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'settings.auditLog.activityLog': 'Aktivitätsprotokoll',
  'settings.auditLog.entriesKeptDaysDays': ' Einträge werden {days} Tage aufbewahrt.',
  'settings.auditLog.whoChangedWhatFrom': 'Wer was geändert hat, von diesem Panel aus (auf diesem Computer oder von einem anderen Gerät) und über <code>kora</code>-Befehle. Passwörter, Lizenzschlüssel und Eingetipptes werden nie aufgezeichnet. Was Koras Administratordienst mit Administratorrechten getan hat (Dienste, Firewall-Regeln, Datenbanken, Updates), steht in einem eigenen Log, das nur Administratoren lesen können: <code2>%ProgramFiles%\\\\Kora\\\\data\\\\audit.log</code2> unter Windows, <code3>/var/log/kora/audit.log</code3> unter Linux.{value}',
  'settings.auditLog.server': 'Server',
  'settings.auditLog.anyServer': 'Beliebiger Server',
  'settings.auditLog.who': 'Wer',
  'settings.auditLog.anyName': 'Beliebiger Name',
  'settings.auditLog.result': 'Ergebnis',
  'settings.auditLog.any': 'Beliebig',
  'settings.auditLog.done': 'Erledigt',
  'settings.auditLog.started': 'Gestartet',
  'settings.auditLog.failed': 'Fehlgeschlagen',
  'settings.auditLog.notAllowed': 'Nicht erlaubt',
  'settings.auditLog.loading': 'Lädt...',
  'settings.auditLog.clearFiltersSeeEverything': 'Lösche die Filter, um alles zu sehen.',
  'settings.auditLog.changesAppearHere': 'Änderungen erscheinen hier.',
  'settings.auditLog.nothingMatchesValue': 'Nichts passt. {value}',
  'settings.auditLog.activityLogEntries': 'Einträge des Aktivitätsprotokolls',
  'settings.auditLog.activityLogNewestFirst': 'Aktivitätsprotokoll, neueste zuerst',
  'settings.auditLog.when': 'Wann',
  'settings.auditLog.what': 'Was',
  'settings.auditLog.showOlderEntries': 'Ältere Einträge anzeigen',
  'settings.auditLog.doneAdministratorRightsValue': 'Mit Administratorrechten erledigt{value}',
  'settings.auditLog.panelAposSEntries': ' Die Einträge des Panels werden {days} Tage aufbewahrt.',
  'settings.auditLog.administratorServicesLogOnly': 'Vom Log des Administratordienstes wird nur angezeigt, was er für dein Konto getan hat. Ein Administrator dieses Computers sieht die Einträge aller.',
  'settings.auditLog.stopped': 'Angehalten',
} satisfies Record<keyof typeof en, string>

register('de', messages)
