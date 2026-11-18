import { register } from '../../index'
import type en from '../../en/core/session'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.session.couldSign': 'Anmeldung nicht möglich.',
} satisfies Record<keyof typeof en, string>

register('de', messages)
