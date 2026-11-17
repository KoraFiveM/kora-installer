import { register } from '../../index'

const messages = {
  'resources.githubSignIn.loaderNodeCheckingGithub': '{loader2, node} Checking GitHub...',
  'resources.githubSignIn.signedIn': 'Signed in',
  'resources.githubSignIn.asUser': 'as {user}',
  'resources.githubSignIn.githubCliInstalled': 'GitHub CLI not installed',
  'resources.githubSignIn.githubCliGhInstalled': 'The GitHub CLI (gh) is not installed. Kora needs it to work with GitHub.',
  'resources.githubSignIn.checkAgain': 'Check again',
  'resources.githubSignIn.morePermissionsNeeded': 'More permissions needed',
  'resources.githubSignIn.signedUser': 'signed in as {user}',
  'resources.githubSignIn.signAgainGiveKora': 'Sign in again to give Kora the permissions it needs.',
  'resources.githubSignIn.notSignedIn': 'Not signed in',
  'resources.githubSignIn.signGithubAgain': 'Sign in to GitHub again',
  'resources.githubSignIn.signGithub': 'Sign in to GitHub',
  'resources.githubSignIn.finishSigningGithub': 'Finish signing in on GitHub',
  'resources.githubSignIn.openNode': 'Open {a, node}',
  'resources.githubSignIn.typeOnetimeCodeCode': 'Type this one-time code: {code2, node} {copyButton, node}',
  'resources.githubSignIn.comeBackHerePage': 'Come back here. This page notices by itself when you have approved it.',
  'resources.githubSignIn.signinDidFinish': 'Sign-in did not finish',
  'resources.githubSignIn.tryAgain': 'Try again',
} as const

register('en', messages)
export default messages
