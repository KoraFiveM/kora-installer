import { register } from '../../index'

const messages = {
  'core.webauthn.panelDidSendPasskey': 'The panel did not send passkey options.',
  'core.webauthn.noPasskeyMade': 'No passkey was made.',
  'core.webauthn.noPasskeyUsed': 'No passkey was used.',
  'core.webauthn.passkeyPromptClosedTimed': 'The passkey prompt was closed, timed out or refused. Try again.',
  'core.webauthn.deviceAlreadyPasskeyHere': 'This device already has a passkey for you here.',
  'core.webauthn.browserWouldUsePasskey': 'The browser would not use a passkey on this address. Open the panel by its name over https://, or sign in with a one-time code.',
  'core.webauthn.browserDeviceCannotMake': 'This browser or device cannot make this kind of passkey. Try another device, or use one-time codes.',
  'core.webauthn.passkeyPromptCancelled': 'The passkey prompt was cancelled.',
  'core.webauthn.passkeyDidWork': 'The passkey did not work.',
} as const

register('en', messages)
export default messages
