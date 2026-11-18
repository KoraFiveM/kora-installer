import { register } from '../../index'

const messages = {
  'errors.toolchainActions.settingUpToolName': 'Setting up {tool, select, git {Git} gh {the GitHub CLI} other {{tool}}}',
  'errors.toolchainActions.startingDatabaseServer': 'Starting the database server',
  'errors.toolchainActions.databaseServerRunning': 'The database server is running.',
  'errors.toolchainActions.signGithub': 'Sign in to GitHub',
  'errors.toolchainActions.startDatabase': 'Start the database',
  'errors.toolchainActions.koraKeepsItsOwn': 'Kora keeps its own copy in its own folder. It is checked against a known checksum first, needs no administrator rights, and changes nothing else on this computer.',
  'errors.toolchainActions.databaseServerInstalledBut': 'The database server is installed but not running. Starting it may need administrator rights.',
  'errors.toolchainActions.close': 'Close',
  'errors.toolchainActions.done': 'Done',
  'errors.toolchainActions.carryingWhatWereDoing': ' Carrying on with what you were doing.',
  'errors.toolchainActions.somethingWaitingTryAgain': ' If something was waiting for it, you can try that again now.',
  'errors.toolchainActions.keepPageOpenCarries': 'You can keep this page open; it carries on by itself.',
  'errors.toolchainActions.checkcircleNodeSignedValue': '{checkCircle2, node}You are signed in.{value}',
  'errors.toolchainActions.installToolName': 'Install {tool, select, git {Git} gh {the GitHub CLI} other {{tool}}}',
  'errors.toolchainActions.toolReady': '{tool, select, git {Git is ready.} gh {The GitHub CLI is ready.} other {{tool} is ready.}}',
} as const

register('en', messages)
export default messages
