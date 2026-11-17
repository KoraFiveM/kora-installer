import { register } from '../../index'
import type en from '../../en/core/errorBoundary'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.errorBoundary.pageNeedsReloaded': 'Diese Seite muss neu geladen werden',
  'core.errorBoundary.koraProbablyUpdatedWhile': 'Kora wurde wahrscheinlich aktualisiert, während diese Seite offen war, deshalb gibt es den angeforderten Teil der Seite nicht mehr. Deine Server sind nicht betroffen. Lade die Seite neu, um die neue Version zu bekommen.',
  'core.errorBoundary.reloadPage': 'Seite neu laden',
  'core.errorBoundary.somethingWentWrong': 'Etwas ist schiefgelaufen',
  'core.errorBoundary.screenHitProblemCould': 'Auf diesem Bildschirm ist ein Problem aufgetreten, deshalb konnte er nicht angezeigt werden. Deine Server sind nicht betroffen.',
  'core.errorBoundary.tryAgain': 'Erneut versuchen',
  'core.errorBoundary.backServers': 'Zurück zu deinen Servern',
} satisfies Record<keyof typeof en, string>

register('de', messages)
