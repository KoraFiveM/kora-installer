import { register } from '../../index'

const messages = {
  'errors.problemActions.tryAgain': 'Try again',
  'errors.problemActions.tryStepAgain': 'Try this step again',
  'errors.problemActions.skipStep': 'Skip this step',
  'errors.problemActions.changeMyAnswers': 'Change my answers',
  'errors.problemActions.signGithubAgain': 'Sign in to GitHub again',
  'errors.problemActions.signGithub': 'Sign in to GitHub',
  'errors.problemActions.installValueMe': 'Install {tool, select, git {Git} gh {the GitHub CLI} mariadb {MariaDB} other {it}} for me',
  'errors.problemActions.installKorasAdministratorServi': "Install Kora's administrator service",
  'errors.problemActions.continueWithoutAdministratorPa': 'Continue without the administrator parts',
  'errors.problemActions.commandRunAdministrator': 'Command to run as administrator',
  'errors.problemActions.commandRun': 'Command to run',
  'errors.problemActions.showLog': 'Show the log',
  'errors.problemActions.copyDetails': 'Copy details',
  'errors.problemActions.startMe': 'Start it for me',
  'errors.problemActions.makeMyOwnCopy': 'Make my own copy (fork)',
  'errors.problemActions.somethingWentWrongPlease': 'Something went wrong. Please try again.',
  'errors.problemActions.openHostof': 'Open {host}',
  'errors.problemActions.changeLicence': 'Change the licence key',
  'errors.problemActions.changeSource': 'Change where the resources come from',
  'errors.problemActions.changeVersions': 'Change which versions to follow',
  'errors.problemActions.changeMode': 'Change where the resources come from',
  'errors.problemActions.changeName': 'Change the name or folder',
  'errors.problemActions.changePorts': 'Change the ports',
  'errors.problemActions.changeBuild': 'Change the server build',
  'errors.problemActions.changeResources': 'Change the resources',
  'errors.problemActions.changeDatabase': 'Change the database choice',
  'errors.problemActions.changeOptions': 'Change the options',
  'errors.problemActions.changeCfx': 'Change the txAdmin sign-in',
  'errors.problemActions.changeGithub': 'Change the GitHub account',
  'errors.problemActions.thePage': 'the page',
} as const

register('en', messages)
export default messages
