import { register } from '../../index'
import type en from '../../en/auth/remoteSignIn'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'auth.remoteSignIn.browserCannotUsePasskeys': 'Dieser Browser kann hier keine Passkeys benutzen. Melde dich stattdessen mit einem Einmalcode an.',
  'auth.remoteSignIn.panelDidStartSignin': 'Das Panel hat die Anmeldung nicht gestartet.',
  'auth.remoteSignIn.yourName': 'Dein Name',
  'auth.remoteSignIn.namePersonWhoRuns': 'Der Name, den dir die Person gegeben hat, die Kora betreibt.',
  'auth.remoteSignIn.password': 'Passwort',
  'auth.remoteSignIn.onetimeCode': 'Einmalcode',
  'auth.remoteSignIn.digitsFromAuthenticatorApp': 'Die 6 Ziffern aus deiner Authenticator-App. Handy verloren? Nimm hier einen deiner Wiederherstellungscodes.',
  'auth.remoteSignIn.signPasskey': 'Mit einem Passkey anmelden ',
  'auth.remoteSignIn.signCode': 'Mit einem Code anmelden ',
  'auth.remoteSignIn.useOnetimeCodeInstead': 'Stattdessen einen Einmalcode benutzen ',
  'auth.remoteSignIn.usePasskeyInstead': 'Stattdessen einen Passkey benutzen ',
  'auth.remoteSignIn.useOnetimeCode': 'Einen Einmalcode benutzen ',
} satisfies Record<keyof typeof en, string>

register('de', messages)
