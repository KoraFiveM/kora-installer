import { register } from '../../index'

const messages = {
  'errors.errorNotice.waitFinishTryAgain': "Wait for it to finish, then try again. Its progress is on the server's Resources tab.",
  'errors.errorNotice.administratorPartStopped': 'The administrator part stopped',
  'errors.errorNotice.korasAdministratorServiceTurne': "Kora's administrator service is not turned on for you",
  'errors.errorNotice.needsAdministratorRights': 'This needs administrator rights',
  'errors.errorNotice.openSystemCheck': 'Open the System check',
  'errors.errorNotice.linkSeeWhatKora': '{link, node} to see what Kora needs on this computer.',
  'errors.errorNotice.installKorasAdministratorServi': "To install Kora's administrator service, open a terminal as administrator on the computer that runs Kora and run this command. It asks for administrator approval once; after that Kora does this without asking.",
  'errors.errorNotice.koraRunningWithoutAdministrato': 'Kora is running without administrator rights, so it cannot do this itself. Run the command below as administrator (on Windows, open a terminal with "Run as administrator"; on Linux, run it as root, for example with sudo), then come back here.',
  'errors.errorNotice.showMeCommand': 'Show me the command',
  'errors.errorNotice.doYourselfOpenTerminal': 'Or do it yourself: open a terminal with "Run as administrator" on the computer that runs Kora, run this command, then come back here.',
  'errors.errorNotice.commandRun': 'Command to run',
  'errors.errorNotice.showDetails': 'Show details',
  'errors.errorNotice.stillStuckSupportbundlelinkNod': 'Still stuck? {supportBundleLink, node} to hand to whoever is helping. Nothing is uploaded.',
  'errors.errorNotice.tryAgain': 'Try again',
  'errors.errorNotice.copyDetails': 'Copy details',
  'errors.errorNotice.logCodeNode': 'The log is in {code, node}',
  'errors.errorNotice.hideLog': 'Hide the log',
  'errors.errorNotice.showLog': 'Show the log',
  'errors.errorNotice.installLog': 'Install log',
  'errors.errorNotice.logCouldRead': 'The log could not be read.',
  'errors.errorNotice.readingLog': 'Reading the log...',
  'errors.errorNotice.savedCodeNode': 'Saved in {code, node}',
  'errors.errorNotice.openSystemCheckNewTab': 'Open the System check in a new tab',
  'errors.errorNotice.koraPanelDidAnswer': 'Kora Panel did not answer. Check that it is still running on the server (`kora panel restart` restarts it), then reload this page.',
} as const

register('en', messages)
export default messages
