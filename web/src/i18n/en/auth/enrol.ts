import { register } from '../../index'

const messages = {
  'auth.enrol.setUpSignin': 'Set up your sign-in',
  'auth.enrol.panelDidStartSetup': 'The panel did not start the set-up.',
  'auth.enrol.myPasskey': 'My passkey',
  'auth.enrol.welcomeUser': 'Welcome, {user}',
  'auth.enrol.checkingLink': 'Checking your link…',
  'auth.enrol.wereInvitedKoraPanel': 'You were invited to Kora Panel as {strong, node}. This link works once. Set up how you will sign in from now on, so nobody needs to pass a secret link around again. ',
  'auth.enrol.passkeyRecommended': 'Passkey (recommended)',
  'auth.enrol.phoneLaptopSecurityKey': 'Your phone, laptop or security key keeps a private key and asks for your fingerprint, face or PIN. Nothing to remember or type.',
  'auth.enrol.nameDevice': 'Name this device',
  'auth.enrol.tellPasskeysApartLater': 'So you can tell your passkeys apart later, for example Work laptop.',
  'auth.enrol.myLaptop': 'My laptop',
  'auth.enrol.setUpPasskey': 'Set up a passkey',
  'auth.enrol.browserCannotMakePasskey': 'This browser cannot make a passkey here.',
  'auth.enrol.valueUsePasswordOnetime': '{value} You can use a password and one-time codes instead.',
  'auth.enrol.passwordOnetimeCodes': 'Password and one-time codes',
  'auth.enrol.worksEverywhereNeedAuthenticat': "Works everywhere. You need an authenticator app (any app that scans a QR code: your phone's, 1Password, Aegis, and so on).",
  'auth.enrol.setUpOnetimeCodes': 'Set up one-time codes',
  'auth.enrol.qrCodeAuthenticatorApp': 'QR code for your authenticator app',
  'auth.enrol.cannotScanTypeKey': 'Cannot scan? Type this key into the app instead:',
  'auth.enrol.setupKey': 'Setup key',
  'auth.enrol.scanAuthenticatorAppImg': 'Scan this with your authenticator app. {img, node}<p>Cannot scan? Type this key into the app instead:</p>{copyField, node}',
  'auth.enrol.choosePassword': 'Choose a password',
  'auth.enrol.leastCharactersFewUnrelated': 'At least 12 characters. A few unrelated words make a strong, easy-to-remember one.',
  'auth.enrol.enterDigitCodeApp': 'Enter the 6-digit code the app shows',
  'auth.enrol.finishSetup': 'Finish set-up',
  'auth.enrol.back': 'Back',
  'auth.enrol.setUpSigned': 'You are set up and signed in',
  'auth.enrol.keepTheseRecoveryCodes': 'Keep these recovery codes somewhere safe, apart from your phone. Each works once, if you lose your authenticator. They are shown only now.',
  'auth.enrol.closePageCarryPanel': 'You can close this page, or carry on to the panel.',
  'auth.enrol.recoveryCodes': 'Recovery codes',
  'auth.enrol.allRecoveryCodes': 'All recovery codes',
  'auth.enrol.iSavedThemCarry': 'I have saved them, carry on',
} as const

register('en', messages)
export default messages
