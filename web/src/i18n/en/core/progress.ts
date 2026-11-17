import { register } from '../../index'

const messages = {
  'core.progress.progress': 'Progress',
} as const

register('en', messages)
export default messages
