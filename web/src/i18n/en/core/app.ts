import { register } from '../../index'

const messages = {
  'core.app.koraNeedsLook': 'Kora needs a look',
} as const

register('en', messages)
export default messages
