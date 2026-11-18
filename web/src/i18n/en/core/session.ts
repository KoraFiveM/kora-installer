import { register } from '../../index'

const messages = {
  'core.session.couldSign': 'Could not sign in.',
} as const

register('en', messages)
export default messages
