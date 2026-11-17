import { register } from '../../index'
import type en from '../../en/auth/reauthProvider'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'auth.reauthProvider.panelDidStartCheck': 'Das Panel hat die Prüfung nicht gestartet.',
  'auth.reauthProvider.confirmItIsYou': 'Bestätige, dass du es bist',
  'auth.reauthProvider.sensitiveActionUsePasskey': 'Das ist eine sensible Aktion. Benutze deinen Passkey oder einen Einmalcode, dann macht Kora weiter.',
  'auth.reauthProvider.useMyPasskey': 'Meinen Passkey benutzen',
  'auth.reauthProvider.useOnetimeCodeInstead': 'Stattdessen einen Einmalcode benutzen',
  'auth.reauthProvider.password': 'Passwort',
  'auth.reauthProvider.onetimeCode': 'Einmalcode',
  'auth.reauthProvider.confirm': 'Bestätigen',
  'auth.reauthProvider.useMyPasskeyInstead': 'Stattdessen meinen Passkey benutzen',
} satisfies Record<keyof typeof en, string>

register('de', messages)
