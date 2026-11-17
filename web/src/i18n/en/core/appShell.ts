import { register } from '../../index'

const messages = {
  'core.appShell.skipMainContent': 'Skip to the main content',
  'core.appShell.main': 'Main',
  'core.appShell.koraAnswering': 'Kora is not answering',
  'core.appShell.koraPanel': 'kora panel',
  'core.appShell.pageCannotReachKora': 'This page cannot reach Kora. Check that <code>kora panel</code> is still running. This page keeps trying.',
  'core.appShell.dismissNotice': 'Dismiss this notice',
  'core.appShell.replaceTokenLink': 'Replace your token link',
  'core.appShell.setUpPasskeyOnetime': 'Set up a passkey or one-time codes',
  'core.appShell.signedTokenSecretLink': 'You signed in with a token: a secret link that works for whoever holds it. <navLink>Set up a passkey or one-time codes</navLink> in Settings; after that the token link stops working for signing in. ',
  'core.appShell.servers': 'Servers',
  'core.appShell.fleet': 'Fleet',
  'core.appShell.installNewServer': 'Install new server',
  'core.appShell.systemCheck': 'System check',
  'core.appShell.settings': 'Settings',
} as const

register('en', messages)
export default messages
