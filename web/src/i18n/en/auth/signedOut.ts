import { register } from '../../index'

const messages = {
  'auth.signedOut.signIn': 'Sign in',
  'auth.signedOut.oldTokenLink': 'Old token link',
  'auth.signedOut.signinLink': 'Sign-in link',
  'auth.signedOut.onlyStillTokenFrom': 'Only if you still have a token from before. It still works for now, and the panel will help you replace it with a passkey.',
  'auth.signedOut.pasteWholeLinkCommand': 'Paste the whole link the command printed. It starts with http://.',
  'auth.signedOut.signIn2': 'Sign in ',
  'auth.signedOut.signKora': 'Sign in to Kora',
  'auth.signedOut.signPasskeySetUp': 'Sign in with the passkey you set up, or with a one-time code. New here? Open the one-time link the person who runs Kora gave you. ',
  'auth.signedOut.signTailscale': 'Sign in with Tailscale',
  'auth.signedOut.administratorRequiresPasskeyOn': 'An administrator requires a passkey or one-time codes here, so the old token link is not accepted.',
  'auth.signedOut.iOldTokenLink': 'I have an old token link',
  'auth.signedOut.koraPanelPrintlink': 'kora panel --print-link',
  'auth.signedOut.koraPanel': 'kora panel',
  'auth.signedOut.keepServersSafePage': 'To keep your servers safe, this page needs a sign-in link. Run <code>kora panel --print-link</code> on this computer to get a new sign-in link. Open that link, or paste it below. If the command says the background service is not installed, run <code2>kora panel</code2> instead and use the link it prints. ',
} as const

register('en', messages)
export default messages
