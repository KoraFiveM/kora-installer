import { register } from '../../index'

const messages = {
  'errors.adminActions.koraInstallerRunningWindows': 'The Kora installer is running. Windows asks for administrator permission in front of this window: choose Yes.',
  'errors.adminActions.installerDidFinishTime': 'The installer did not finish in time, or the permission was not given.',
  'errors.adminActions.koraInstallerRunningWindows2': 'The Kora installer is running. Windows asks for administrator permission on the computer that runs Kora: choose Yes.',
  'errors.adminActions.installerStopped': 'The installer stopped.',
  'errors.adminActions.installerDidFinishTime2': 'The installer did not finish in time. Check the computer that runs Kora.',
  'errors.adminActions.korasAdministratorServiceInsta': "Kora's administrator service is installed. Kora can now do this without asking.",
  'errors.adminActions.korasAdministratorServiceInsta2': "Kora's administrator service was not installed.",
} as const

register('en', messages)
export default messages
