import { register } from '../../index'

const messages = {
  'core.ago.justNow': 'just now',
} as const

register('en', messages)
export default messages
