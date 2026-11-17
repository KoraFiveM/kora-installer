import { register } from '../../index'

const messages = {
  'core.client.serverAnsweredUnexpectedError': 'The server answered with an unexpected error ({status}).',
  'core.client.couldReachKoraCheck': 'Could not reach Kora. Check that it is still running, then try again.',
  'core.client.baseServersEncodeuricomponentB': '{BASE}/servers/{encodeURIComponent}/backups/{encodeURIComponent2}/file',
} as const

register('en', messages)
export default messages
