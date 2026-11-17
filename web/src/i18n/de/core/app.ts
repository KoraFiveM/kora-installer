import { register } from '../../index'
import type en from '../../en/core/app'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.app.koraNeedsLook': 'Kora braucht einen Blick',
} satisfies Record<keyof typeof en, string>

register('de', messages)
