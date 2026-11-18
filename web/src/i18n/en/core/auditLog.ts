import { register } from '../../index'

const messages = {
  'core.auditLog.done': 'Done',
  'core.auditLog.started': 'Started',
  'core.auditLog.failed': 'Failed',
  'core.auditLog.notAllowed': 'Not allowed',
} as const

register('en', messages)
export default messages
