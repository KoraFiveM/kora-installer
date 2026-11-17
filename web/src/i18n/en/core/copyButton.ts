import { register } from '../../index'

const messages = {
  'core.copyButton.pressCtrlC': 'Press Ctrl+C',
  'core.copyButton.copied': 'Copied',
} as const

register('en', messages)
export default messages
