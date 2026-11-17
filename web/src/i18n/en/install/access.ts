import { register } from '../../index'

const messages = {
  'install.access.needsAdministratorAccount': 'Needs an administrator account.',
} as const

register('en', messages)
export default messages
