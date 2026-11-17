import { register } from '../../index'
import type en from '../../en/errors/adminActions'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'errors.adminActions.koraInstallerRunningWindows': 'Das Kora-Installationsprogramm läuft. Windows fragt vor diesem Fenster nach Administratorrechten: Wähle Ja.',
  'errors.adminActions.installerDidFinishTime': 'Das Installationsprogramm wurde nicht rechtzeitig fertig, oder die Erlaubnis wurde nicht erteilt.',
  'errors.adminActions.koraInstallerRunningWindows2': 'Das Kora-Installationsprogramm läuft. Windows fragt auf dem Computer, auf dem Kora läuft, nach Administratorrechten: Wähle Ja.',
  'errors.adminActions.installerStopped': 'Das Installationsprogramm wurde angehalten.',
  'errors.adminActions.installerDidFinishTime2': 'Das Installationsprogramm wurde nicht rechtzeitig fertig. Sieh auf dem Computer nach, auf dem Kora läuft.',
  'errors.adminActions.korasAdministratorServiceInsta': 'Koras Administratordienst ist installiert. Kora kann das jetzt ohne Nachfrage tun.',
  'errors.adminActions.korasAdministratorServiceInsta2': 'Koras Administratordienst wurde nicht installiert.',
} satisfies Record<keyof typeof en, string>

register('de', messages)
