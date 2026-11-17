import { register } from '../../index'

const messages = {
  'core.announcer.labelMPerCent': '{label}: {m} per cent',
} as const

register('en', messages)
export default messages
