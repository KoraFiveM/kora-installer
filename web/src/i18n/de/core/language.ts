import { register } from '../../index'
import type en from '../../en/core/language'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.language.label': 'Sprache',
  'core.language.beta': '{name} (Beta)',
  'core.language.hint': 'Beta: maschinell unterstützt und noch nicht von Muttersprachlern geprüft. Texte, die noch nicht übersetzt sind, erscheinen auf Englisch.',
  'core.language.remembered': 'Die Auswahl wird von diesem Browser gespeichert. Einige Texte (die Befehlszeile, Logs und manche Meldungen des Servers) gibt es nur auf Englisch.',
} satisfies Record<keyof typeof en, string>

register('de', messages)
