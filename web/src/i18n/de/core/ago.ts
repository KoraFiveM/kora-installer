import { register } from '../../index'
import type en from '../../en/core/ago'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.ago.justNow': 'gerade eben',
} satisfies Record<keyof typeof en, string>

register('de', messages)
