import { register } from '../../index'
import type en from '../../en/core/confirmDialog'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.confirmDialog.cancel': 'Abbrechen',
} satisfies Record<keyof typeof en, string>

register('de', messages)
