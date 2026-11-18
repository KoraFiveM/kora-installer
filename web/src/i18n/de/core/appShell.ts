import { register } from '../../index'
import type en from '../../en/core/appShell'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.appShell.skipMainContent': 'Zum Hauptinhalt springen',
  'core.appShell.main': 'Hauptmenü',
  'core.appShell.koraAnswering': 'Kora antwortet nicht',
  'core.appShell.pageCannotReachKora': 'Diese Seite erreicht Kora nicht. Prüfe, ob <code>kora panel</code> noch läuft. Diese Seite versucht es weiter.',
  'core.appShell.dismissNotice': 'Diesen Hinweis schließen',
  'core.appShell.replaceTokenLink': 'Ersetze deinen Token-Link',
  'core.appShell.signedTokenSecretLink': 'Du hast dich mit einem Token angemeldet: einem geheimen Link, der für jeden funktioniert, der ihn besitzt. <navLink>Richte in den Einstellungen einen Passkey oder Einmalcodes ein</navLink>; danach funktioniert der Token-Link nicht mehr zum Anmelden. ',
  'core.appShell.servers': 'Server',
  'core.appShell.fleet': 'Flotte',
  'core.appShell.installNewServer': 'Neuen Server installieren',
  'core.appShell.systemCheck': 'Systemprüfung',
  'core.appShell.settings': 'Einstellungen',
} satisfies Record<keyof typeof en, string>

register('de', messages)
