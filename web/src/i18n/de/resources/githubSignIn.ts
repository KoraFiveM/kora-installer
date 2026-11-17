import { register } from '../../index'
import type en from '../../en/resources/githubSignIn'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'resources.githubSignIn.loaderNodeCheckingGithub': '{loader2, node} GitHub wird geprüft...',
  'resources.githubSignIn.signedIn': 'Angemeldet',
  'resources.githubSignIn.asUser': 'als {user}',
  'resources.githubSignIn.githubCliInstalled': 'GitHub-CLI nicht installiert',
  'resources.githubSignIn.githubCliGhInstalled': 'Die GitHub-CLI (gh) ist nicht installiert. Kora braucht sie, um mit GitHub zu arbeiten.',
  'resources.githubSignIn.checkAgain': 'Erneut prüfen',
  'resources.githubSignIn.morePermissionsNeeded': 'Weitere Berechtigungen nötig',
  'resources.githubSignIn.signedUser': 'angemeldet als {user}',
  'resources.githubSignIn.signAgainGiveKora': 'Melde dich erneut an, um Kora die nötigen Berechtigungen zu geben.',
  'resources.githubSignIn.notSignedIn': 'Nicht angemeldet',
  'resources.githubSignIn.signGithubAgain': 'Erneut bei GitHub anmelden',
  'resources.githubSignIn.signGithub': 'Bei GitHub anmelden',
  'resources.githubSignIn.finishSigningGithub': 'Anmeldung bei GitHub abschließen',
  'resources.githubSignIn.openNode': '{a, node} öffnen',
  'resources.githubSignIn.typeOnetimeCodeCode': 'Gib diesen Einmalcode ein: {code2, node} {copyButton, node}',
  'resources.githubSignIn.comeBackHerePage': 'Komm hierher zurück. Diese Seite merkt von selbst, wenn du es bestätigt hast.',
  'resources.githubSignIn.signinDidFinish': 'Die Anmeldung wurde nicht abgeschlossen',
  'resources.githubSignIn.tryAgain': 'Erneut versuchen',
} satisfies Record<keyof typeof en, string>

register('de', messages)
