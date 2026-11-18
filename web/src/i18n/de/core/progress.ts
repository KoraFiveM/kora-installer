import { register } from '../../index'
import type en from '../../en/core/progress'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.progress.progress': 'Fortschritt',
} satisfies Record<keyof typeof en, string>

register('de', messages)
