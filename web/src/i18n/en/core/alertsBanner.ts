import { register } from '../../index'

const messages = {
  'core.alertsBanner.serverAlerts': 'Server alerts',
  'core.alertsBanner.dismissAlertServer': 'Dismiss the alert for {server}',
  'core.alertsBanner.dismiss': 'Dismiss',
  'core.alertsBanner.alertCouldDismissed': 'The alert could not be dismissed',
  'core.alertsBanner.valueMoreAlerts': 'And {count} more alerts.',
} as const

register('en', messages)
export default messages
