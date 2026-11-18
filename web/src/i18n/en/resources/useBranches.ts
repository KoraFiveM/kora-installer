import { register } from '../../index'

const messages = {
  'resources.useBranches.normalVersionDefaultBranch': 'Normal version ({default_branch})',
  'resources.useBranches.fixedVersion': 'Fixed version',
  'resources.useBranches.notAvailable': 'Not available',
} as const

register('en', messages)
export default messages
