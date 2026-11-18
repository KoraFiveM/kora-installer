import { register } from '../../index'
import type en from '../../en/core/supportBundleLink'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.supportBundleLink.createSupportBundle': 'Support-Paket erstellen',
} satisfies Record<keyof typeof en, string>

register('de', messages)
