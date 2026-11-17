import { register } from '../../index'
import type en from '../../en/core/utils'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.utils.portsMustWholeNumbers': 'Ports müssen ganze Zahlen von 1024 bis 65535 sein.',
  'core.utils.gamePortTxadminPort': 'Der Spiel-Port und der txAdmin-Port müssen verschieden sein.',
} satisfies Record<keyof typeof en, string>

register('de', messages)
