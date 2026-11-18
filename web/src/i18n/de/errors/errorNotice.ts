import { register } from '../../index'
import type en from '../../en/errors/errorNotice'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'errors.errorNotice.waitFinishTryAgain': 'Warte, bis es fertig ist, und versuche es dann erneut. Der Fortschritt steht im Tab Resources des Servers.',
  'errors.errorNotice.administratorPartStopped': 'Der Administrator-Teil wurde angehalten',
  'errors.errorNotice.korasAdministratorServiceTurne': 'Koras Administratordienst ist für dich nicht eingeschaltet',
  'errors.errorNotice.needsAdministratorRights': 'Dafür sind Administratorrechte nötig',
  'errors.errorNotice.openSystemCheck': 'Systemprüfung öffnen',
  'errors.errorNotice.linkSeeWhatKora': '{link, node}, um zu sehen, was Kora auf diesem Computer braucht.',
  'errors.errorNotice.installKorasAdministratorServi': 'Um Koras Administratordienst zu installieren, öffne auf dem Computer, auf dem Kora läuft, ein Terminal als Administrator und führe diesen Befehl aus. Er fragt einmal nach der Administrator-Freigabe; danach erledigt Kora das ohne Nachfrage.',
  'errors.errorNotice.koraRunningWithoutAdministrato': 'Kora läuft ohne Administratorrechte und kann das deshalb nicht selbst tun. Führe den folgenden Befehl als Administrator aus (öffne unter Windows ein Terminal mit „Als Administrator ausführen“; führe ihn unter Linux als root aus, zum Beispiel mit sudo) und komm dann hierher zurück.',
  'errors.errorNotice.showMeCommand': 'Befehl anzeigen',
  'errors.errorNotice.doYourselfOpenTerminal': 'Oder mach es selbst: Öffne auf dem Computer, auf dem Kora läuft, ein Terminal mit „Als Administrator ausführen“, führe diesen Befehl aus und komm dann hierher zurück.',
  'errors.errorNotice.commandRun': 'Auszuführender Befehl',
  'errors.errorNotice.showDetails': 'Details anzeigen',
  'errors.errorNotice.stillStuckSupportbundlelinkNod': 'Immer noch festgefahren? {supportBundleLink, node}, um es der Person zu geben, die dir hilft. Es wird nichts hochgeladen.',
  'errors.errorNotice.tryAgain': 'Erneut versuchen',
  'errors.errorNotice.copyDetails': 'Details kopieren',
  'errors.errorNotice.logCodeNode': 'Das Log liegt in {code, node}',
  'errors.errorNotice.hideLog': 'Log ausblenden',
  'errors.errorNotice.showLog': 'Log anzeigen',
  'errors.errorNotice.installLog': 'Installations-Log',
  'errors.errorNotice.logCouldRead': 'Das Log konnte nicht gelesen werden.',
  'errors.errorNotice.readingLog': 'Log wird gelesen...',
  'errors.errorNotice.savedCodeNode': 'Gespeichert in {code, node}',
  'errors.errorNotice.openSystemCheckNewTab': 'Systemprüfung in einem neuen Tab öffnen',
  'errors.errorNotice.koraPanelDidAnswer': 'Kora Panel hat nicht geantwortet. Prüfe, ob es auf dem Server noch läuft (`kora panel restart` startet es neu), und lade diese Seite dann neu.',
} satisfies Record<keyof typeof en, string>

register('de', messages)
