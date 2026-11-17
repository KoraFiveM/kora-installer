import { register } from '../../index'
import type en from '../../en/core/copyButton'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.copyButton.pressCtrlC': 'Drücke Strg+C',
  'core.copyButton.copied': 'Kopiert',
} satisfies Record<keyof typeof en, string>

register('de', messages)
