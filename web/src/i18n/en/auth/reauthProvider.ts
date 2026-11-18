import { register } from '../../index'

const messages = {
  'auth.reauthProvider.panelDidStartCheck': 'The panel did not start the check.',
  'auth.reauthProvider.confirmItIsYou': 'Confirm it is you',
  'auth.reauthProvider.sensitiveActionUsePasskey': 'This is a sensitive action. Use your passkey or a one-time code, and Kora carries on.',
  'auth.reauthProvider.useMyPasskey': 'Use my passkey',
  'auth.reauthProvider.useOnetimeCodeInstead': 'Use a one-time code instead',
  'auth.reauthProvider.password': 'Password',
  'auth.reauthProvider.onetimeCode': 'One-time code',
  'auth.reauthProvider.confirm': 'Confirm',
  'auth.reauthProvider.useMyPasskeyInstead': 'Use my passkey instead',
} as const

register('en', messages)
export default messages
