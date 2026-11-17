import { register } from '../../index'

const messages = {
  'core.useJob.koraLostContactJob': 'Kora lost contact with this job. It may still be running: reload the page to look again.',
} as const

register('en', messages)
export default messages
