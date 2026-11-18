import { register } from '../../index'
import type en from '../../en/core/alertsBanner'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.alertsBanner.serverAlerts': 'Server-Warnungen',
  'core.alertsBanner.dismissAlertServer': 'Warnung für {server} schließen',
  'core.alertsBanner.dismiss': 'Schließen',
  'core.alertsBanner.alertCouldDismissed': 'Die Warnung konnte nicht geschlossen werden',
  'core.alertsBanner.valueMoreAlerts': 'Und {count} weitere Warnungen.',
  'core.alertsBanner.whatDo': 'Was zu tun ist',
  'core.alertsBanner.dismissAlertTitle': 'Warnung schließen: {title}',
} satisfies Record<keyof typeof en, string>

register('de', messages)
