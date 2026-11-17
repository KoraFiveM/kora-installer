import { register } from '../../index'

const messages = {
  'settings.auditLog.activityLog': 'Activity log',
  'settings.auditLog.entriesKeptDaysDays': ' Entries are kept for {days} days.',
  'settings.auditLog.whoChangedWhatFrom': "Who changed what, from this panel (on this computer or from another device) and from <code>kora</code> commands. Passwords, licence keys and what was typed are never recorded. What Kora's administrator service did with administrator rights (services, firewall rules, databases, updates, scheduled backups) is kept in its own log, <code2>%ProgramFiles%\\\\Kora\\\\data\\\\audit.log</code2> on Windows and <code3>/var/log/kora/audit.log</code3> on Linux. Those entries are shown here too, marked “Done with administrator rights”.{value}",
  'settings.auditLog.server': 'Server',
  'settings.auditLog.anyServer': 'Any server',
  'settings.auditLog.who': 'Who',
  'settings.auditLog.anyName': 'Any name',
  'settings.auditLog.result': 'Result',
  'settings.auditLog.any': 'Any',
  'settings.auditLog.done': 'Done',
  'settings.auditLog.started': 'Started',
  'settings.auditLog.failed': 'Failed',
  'settings.auditLog.notAllowed': 'Not allowed',
  'settings.auditLog.loading': 'Loading...',
  'settings.auditLog.clearFiltersSeeEverything': 'Clear the filters to see everything.',
  'settings.auditLog.changesAppearHere': 'Changes will appear here.',
  'settings.auditLog.nothingMatchesValue': 'Nothing matches. {value}',
  'settings.auditLog.activityLogEntries': 'Activity log entries',
  'settings.auditLog.activityLogNewestFirst': 'Activity log, newest first',
  'settings.auditLog.when': 'When',
  'settings.auditLog.what': 'What',
  'settings.auditLog.showOlderEntries': 'Show older entries',
  'settings.auditLog.doneAdministratorRightsValue': 'Done with administrator rights{value}',
  'settings.auditLog.panelAposSEntries': " The panel's entries are kept for {days} days.",
  'settings.auditLog.administratorServicesLogOnly': "Of the administrator service's log, only what it did for your account is shown. An administrator of this computer sees everyone's.",
  'settings.auditLog.stopped': 'Stopped',
} as const

register('en', messages)
export default messages
