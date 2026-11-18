import { register } from '../../index'
import type en from '../../en/resources/updateProgress'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'resources.updateProgress.rollingBack': 'Wird zurückgesetzt',
} satisfies Record<keyof typeof en, string>

register('de', messages)
