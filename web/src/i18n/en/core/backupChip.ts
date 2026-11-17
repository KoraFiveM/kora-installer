import { register } from '../../index'

const messages = {
  'core.backupChip.textNeedsLook': '{text} (needs a look)',
} as const

register('en', messages)
export default messages
