import { register } from '../../index'
import type en from '../../en/settings/contribute'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'settings.contribute.title': 'Zu Kora beitragen',
  'settings.contribute.intro': 'Fehler gefunden oder etwas zu ändern? Du kannst es als Pull Request (PR) im Kora-Repository vorschlagen. Du brauchst ein GitHub-Konto, das du hast, wenn du dich oben angemeldet hast.',
  'settings.contribute.step1': 'Öffne das Kora-Repository auf GitHub und klicke auf Fork. So entsteht deine eigene Kopie.',
  'settings.contribute.step2': 'Mache die Änderung in deinem Fork auf einem neuen Branch.',
  'settings.contribute.step3': 'Klicke in deinem Fork auf Contribute, dann auf Open pull request, und sende ihn an den Branch main des Kora-Repositorys.',
  'settings.contribute.openRepo': 'Kora-Repository öffnen',
  'settings.contribute.guide': 'Beitragsleitfaden lesen',
} as const satisfies Record<keyof typeof en, string>

register('de', messages)
export default messages
