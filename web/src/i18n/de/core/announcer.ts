import { register } from '../../index'
import type en from '../../en/core/announcer'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.announcer.labelMPerCent': '{label}: {m} Prozent',
} satisfies Record<keyof typeof en, string>

register('de', messages)
