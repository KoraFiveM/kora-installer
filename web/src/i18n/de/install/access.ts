import { register } from '../../index'
import type en from '../../en/install/access'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'install.access.needsAdministratorAccount': 'Dafür ist ein Administratorkonto nötig.',
} satisfies Record<keyof typeof en, string>

register('de', messages)
