import { register } from '../../index'
import type en from '../../en/resources/useBranches'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'resources.useBranches.normalVersionDefaultBranch': 'Normale Version ({default_branch})',
  'resources.useBranches.fixedVersion': 'Feste Version',
  'resources.useBranches.notAvailable': 'Nicht verfügbar',
} satisfies Record<keyof typeof en, string>

register('de', messages)
