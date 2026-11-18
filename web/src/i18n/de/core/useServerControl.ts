import { register } from '../../index'
import type en from '../../en/core/useServerControl'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.useServerControl.actionDidFinish': 'Die Aktion wurde nicht abgeschlossen.',
  'core.useServerControl.actionDidStart': 'Die Aktion wurde nicht gestartet.',
  'core.useServerControl.starting': 'Startet',
  'core.useServerControl.stopping': 'Hält an',
  'core.useServerControl.restarting': 'Startet neu',
} satisfies Record<keyof typeof en, string>

register('de', messages)
