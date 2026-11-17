import { register } from '../../index'
import type en from '../../en/core/alertsBanner'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.alertsBanner.serverAlerts': 'Server-Warnungen',
  'core.alertsBanner.dismissAlertServer': 'Warnung für {server} schließen',
  'core.alertsBanner.dismiss': 'Schließen',
  'core.alertsBanner.alertCouldDismissed': 'Die Warnung konnte nicht geschlossen werden',
  'core.alertsBanner.valueMoreAlerts': 'Und {count} weitere Warnungen.',
} satisfies Record<keyof typeof en, string>

register('de', messages)
