import { register } from '../../index'
import type en from '../../en/core/jobHistory'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.jobHistory.openInstallCarry': 'Öffne die Installation, um weiterzumachen',
  'core.jobHistory.recentJobs': 'Letzte Jobs',
  'core.jobHistory.updatesInstallsOtherJobs': 'Updates, Installationen und andere Jobs, die du gestartet hast. Sie bleiben erhalten, wenn Kora Panel neu startet. Ein Job, der beim Anhalten des Panels noch lief, erscheint als durch einen Neustart gestoppt, mit dem nächsten Schritt.',
  'core.jobHistory.loading': 'Lädt...',
  'core.jobHistory.nothingRunYet': 'Bisher ist nichts gelaufen.',
  'core.jobHistory.running': 'Läuft',
  'core.jobHistory.finished': 'Abgeschlossen',
  'core.jobHistory.didFinish': 'Nicht abgeschlossen',
  'core.jobHistory.stoppedRestart': 'Durch Neustart gestoppt',
} satisfies Record<keyof typeof en, string>

register('de', messages)
