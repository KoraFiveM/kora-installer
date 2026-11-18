import { register } from '../../index'
import type en from '../../en/resources/versionBadge'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'resources.versionBadge.notAvailable': 'Nicht verfügbar',
} satisfies Record<keyof typeof en, string>

register('de', messages)
