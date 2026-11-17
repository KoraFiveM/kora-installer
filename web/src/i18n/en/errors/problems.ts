import { register } from '../../index'

// Messages the SERVER names by id (a Problem's `i18n`, see docs/translations.md). The English here is exactly what the server
// writes in `message`, so the panel in English shows the same words as before. The ids are the contract with crates/kora-core.
const messages = {
  'errors.problems.ghMissing': 'The GitHub CLI (gh) is not installed. Kora needs it to work with GitHub.',
  'errors.problems.gitMissing': 'Git is not installed. Kora needs it to download and update resources.',
  'errors.problems.dbStopped': 'The database server {name} is installed but not running.',
  'errors.problems.dbStoppedStep': 'The database server {name} is installed but not running. Start it, then try this step again.',
  'errors.problems.githubSignedOut': 'You are not signed in to GitHub.',
  'errors.problems.githubMissingScopes': 'Your GitHub sign-in does not give Kora all the permissions it needs. Sign in again to give them.',
  'errors.problems.dbNeedsAdmin': 'Installing the database server needs administrator rights.',
  'errors.problems.gitNeedsAdmin': 'Git is not installed, and installing it here needs administrator rights.',
  'errors.problems.licenceMissing': 'a Cfx.re license key is required (get one at https://portal.cfx.re/servers/registration-keys)',
  'errors.problems.githubSignInNotStarted': 'Kora could not start the GitHub sign-in.',
  'errors.problems.githubSignInNotCompleted': 'The GitHub sign-in was not completed (the code expired or was refused).',
  'errors.problems.githubSignInUnfinished': 'The GitHub sign-in did not finish.',
} as const

register('en', messages)
export default messages
