import { register } from '../../index'
import type en from '../../en/errors/problems'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'errors.problems.ghMissing': 'Die GitHub-CLI (gh) ist nicht installiert. Kora braucht sie, um mit GitHub zu arbeiten.',
  'errors.problems.gitMissing': 'Git ist nicht installiert. Kora braucht es, um Resources herunterzuladen und zu aktualisieren.',
  'errors.problems.dbStopped': 'Der Datenbankserver {name} ist installiert, läuft aber nicht.',
  'errors.problems.dbStoppedStep': 'Der Datenbankserver {name} ist installiert, läuft aber nicht. Starte ihn und versuche diesen Schritt dann erneut.',
  'errors.problems.githubSignedOut': 'Du bist nicht bei GitHub angemeldet.',
  'errors.problems.githubMissingScopes': 'Deine GitHub-Anmeldung gibt Kora nicht alle Berechtigungen, die es braucht. Melde dich erneut an, um sie zu erteilen.',
  'errors.problems.dbNeedsAdmin': 'Für die Installation des Datenbankservers sind Administratorrechte nötig.',
  'errors.problems.gitNeedsAdmin': 'Git ist nicht installiert, und die Installation hier braucht Administratorrechte.',
  'errors.problems.licenceMissing': 'ein Cfx.re-Lizenzschlüssel ist erforderlich (du bekommst einen unter https://portal.cfx.re/servers/registration-keys)',
  'errors.problems.githubSignInNotStarted': 'Kora konnte die GitHub-Anmeldung nicht starten.',
  'errors.problems.githubSignInNotCompleted': 'Die GitHub-Anmeldung wurde nicht abgeschlossen (der Code ist abgelaufen oder wurde abgelehnt).',
  'errors.problems.githubSignInUnfinished': 'Die GitHub-Anmeldung wurde nicht abgeschlossen.',
} satisfies Record<keyof typeof en, string>

register('de', messages)
