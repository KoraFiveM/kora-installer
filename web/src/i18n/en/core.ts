import { register } from '../index'

// Words and phrases used everywhere: the page frame, common buttons, times and sizes.
const messages = {
  'core.skip': 'Skip to the main content',
  'core.timeJustNow': 'just now',
} as const

register('en', messages)
export default messages
