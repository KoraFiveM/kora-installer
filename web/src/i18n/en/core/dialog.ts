import { register } from '../../index'

const messages = {
  'core.dialog.close': 'Close',
} as const

register('en', messages)
export default messages
