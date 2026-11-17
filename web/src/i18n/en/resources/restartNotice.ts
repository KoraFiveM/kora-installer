import { register } from '../../index'

const messages = {
  'resources.restartNotice.whatChangedWhy': 'What changed: {why}.',
  'resources.restartNotice.nodeNodeRestartResource': '{node, node} Restart this resource ',
  'resources.restartNotice.restartWholeServer': 'Restart the whole server',
  'resources.restartNotice.restartNeededRunningServer': 'Restart needed: the running server still has the old version.',
} as const

register('en', messages)
export default messages
