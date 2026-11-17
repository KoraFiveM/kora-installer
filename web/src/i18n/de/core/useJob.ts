import { register } from '../../index'
import type en from '../../en/core/useJob'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.useJob.koraLostContactJob': 'Kora hat den Kontakt zu diesem Job verloren. Er läuft vielleicht noch: Lade die Seite neu, um nachzusehen.',
} satisfies Record<keyof typeof en, string>

register('de', messages)
