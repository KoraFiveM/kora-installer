import { register } from '../../index'

const messages = {
  'core.useServerControl.actionDidFinish': 'The action did not finish.',
  'core.useServerControl.actionDidStart': 'The action did not start.',
  'core.useServerControl.starting': 'Starting',
  'core.useServerControl.stopping': 'Stopping',
  'core.useServerControl.restarting': 'Restarting',
} as const

register('en', messages)
export default messages
