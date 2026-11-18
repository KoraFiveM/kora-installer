import { register } from '../../index'
import type en from '../../en/core/dialog'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.dialog.close': 'Schließen',
} satisfies Record<keyof typeof en, string>

register('de', messages)
