import { register } from '../../index'
import type en from '../../en/servers/scheduleTab'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'servers.scheduleTab.automaticUpdatesOff': 'Automatische Updates sind aus.',
  'servers.scheduleTab.savedServerUpdatesItself': 'Gespeichert. Der Server aktualisiert sich selbst jeden {day} um {time}.',
  'servers.scheduleTab.automaticUpdates': 'Automatische Updates',
  'servers.scheduleTab.everyServerUpdateItself': 'Jeder Server kann sich nach Zeitplan selbst aktualisieren. Kora prüft auf neuere Versionen (veröffentlichte Versionen oder die neuesten Änderungen, je nachdem, wem der Server folgt), aktualisiert, was sicher zu aktualisieren ist, und prüft, ob der Server noch startet. Es ist aus, bis du es einschaltest.',
  'servers.scheduleTab.currentSchedule': 'Aktueller Zeitplan',
  'servers.scheduleTab.howOften': 'Wie oft',
  'servers.scheduleTab.off': 'Aus',
  'servers.scheduleTab.updatesOnlyHappenWhen': 'Updates passieren nur, wenn du sie selbst ausführst.',
  'servers.scheduleTab.everyDay': 'Jeden Tag',
  'servers.scheduleTab.onceWeek': 'Einmal pro Woche',
  'servers.scheduleTab.day': 'Tag',
  'servers.scheduleTab.timeHours': 'Uhrzeit (24 Stunden)',
  'servers.scheduleTab.useFormHhMm': 'Verwende die Form HH:MM, zum Beispiel 04:00.',
  'servers.scheduleTab.runUpdate': 'Das Update ausführen als',
  'servers.scheduleTab.you': 'Du',
  'servers.scheduleTab.usesGithubSigninPrivate': 'Nutzt deine GitHub-Anmeldung, damit auch private Forks geholt werden können. Läuft, solange dein Konto Hintergrundaufgaben ausführen kann.',
  'servers.scheduleTab.systemAccount': 'Das Systemkonto',
  'servers.scheduleTab.runsEvenWhenNobody': 'Läuft auch, wenn niemand angemeldet ist, hat aber keine GitHub-Anmeldung, deshalb können private Forks nicht geholt werden. Zum Einrichten sind Administratorrechte nötig.',
  'servers.scheduleTab.turnScheduleOff': 'Zeitplan ausschalten',
  'servers.scheduleTab.saveSchedule': 'Zeitplan speichern',
  'servers.scheduleTab.savedServerUpdatesDaily': 'Gespeichert. Der Server aktualisiert sich selbst jeden Tag um {time}.',
} satisfies Record<keyof typeof en, string>

register('de', messages)
