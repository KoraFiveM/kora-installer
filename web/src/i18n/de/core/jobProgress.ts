import { register } from '../../index'
import type en from '../../en/core/jobProgress'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.jobProgress.allStepsFinished': 'Alle Schritte sind abgeschlossen.',
  'core.jobProgress.overallProgress': 'Gesamtfortschritt',
  'core.jobProgress.safetyCopySavedId': 'Sicherheitskopie gespeichert ({id}). Du kannst darauf zurückgehen.',
  'core.jobProgress.countSizeResourcesFinished': '{count} von {size} Resources abgeschlossen',
  'core.jobProgress.waiting': 'Wartet',
  'core.jobProgress.inProgress': 'Läuft',
  'core.jobProgress.done': 'Erledigt',
  'core.jobProgress.skipped': 'Übersprungen',
  'core.jobProgress.failed': 'Fehlgeschlagen',
} satisfies Record<keyof typeof en, string>

register('de', messages)
