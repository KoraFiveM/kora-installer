import { register } from '../../index'
import type en from '../../en/core/webauthn'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.webauthn.panelDidSendPasskey': 'Das Panel hat keine Passkey-Optionen gesendet.',
  'core.webauthn.noPasskeyMade': 'Es wurde kein Passkey erstellt.',
  'core.webauthn.noPasskeyUsed': 'Es wurde kein Passkey verwendet.',
  'core.webauthn.passkeyPromptClosedTimed': 'Die Passkey-Abfrage wurde geschlossen, ist abgelaufen oder wurde abgelehnt. Versuche es erneut.',
  'core.webauthn.deviceAlreadyPasskeyHere': 'Dieses Gerät hat hier schon einen Passkey für dich.',
  'core.webauthn.browserWouldUsePasskey': 'Der Browser wollte auf dieser Adresse keinen Passkey verwenden. Öffne das Panel über seinen Namen mit https:// oder melde dich mit einem Einmalcode an.',
  'core.webauthn.browserDeviceCannotMake': 'Dieser Browser oder dieses Gerät kann diese Art Passkey nicht erstellen. Versuche ein anderes Gerät oder nutze Einmalcodes.',
  'core.webauthn.passkeyPromptCancelled': 'Die Passkey-Abfrage wurde abgebrochen.',
  'core.webauthn.passkeyDidWork': 'Der Passkey hat nicht funktioniert.',
} satisfies Record<keyof typeof en, string>

register('de', messages)
