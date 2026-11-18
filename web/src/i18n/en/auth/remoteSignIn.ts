import { register } from '../../index'

const messages = {
  'auth.remoteSignIn.browserCannotUsePasskeys': 'This browser cannot use passkeys here. Sign in with a one-time code instead.',
  'auth.remoteSignIn.panelDidStartSignin': 'The panel did not start the sign-in.',
  'auth.remoteSignIn.yourName': 'Your name',
  'auth.remoteSignIn.namePersonWhoRuns': 'The name the person who runs Kora gave you.',
  'auth.remoteSignIn.password': 'Password',
  'auth.remoteSignIn.onetimeCode': 'One-time code',
  'auth.remoteSignIn.digitsFromAuthenticatorApp': 'The 6 digits from your authenticator app. Lost your phone? Use one of your recovery codes here.',
  'auth.remoteSignIn.signPasskey': 'Sign in with a passkey ',
  'auth.remoteSignIn.signCode': 'Sign in with a code ',
  'auth.remoteSignIn.useOnetimeCodeInstead': 'Use a one-time code instead ',
  'auth.remoteSignIn.usePasskeyInstead': 'Use a passkey instead ',
  'auth.remoteSignIn.useOnetimeCode': 'Use a one-time code ',
} as const

register('en', messages)
export default messages
