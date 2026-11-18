import { register } from '../../index'
import type en from '../../en/errors/problemActions'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'errors.problemActions.tryAgain': 'Erneut versuchen',
  'errors.problemActions.tryStepAgain': 'Diesen Schritt erneut versuchen',
  'errors.problemActions.skipStep': 'Diesen Schritt überspringen',
  'errors.problemActions.changeMyAnswers': 'Meine Antworten ändern',
  'errors.problemActions.signGithubAgain': 'Erneut bei GitHub anmelden',
  'errors.problemActions.signGithub': 'Bei GitHub anmelden',
  'errors.problemActions.installValueMe': '{tool, select, git {Git} gh {Die GitHub-CLI} mariadb {MariaDB} other {Es}} für mich installieren',
  'errors.problemActions.installKorasAdministratorServi': 'Koras Administratordienst installieren',
  'errors.problemActions.continueWithoutAdministratorPa': 'Ohne die Administrator-Teile weitermachen',
  'errors.problemActions.commandRunAdministrator': 'Als Administrator auszuführender Befehl',
  'errors.problemActions.commandRun': 'Auszuführender Befehl',
  'errors.problemActions.showLog': 'Log anzeigen',
  'errors.problemActions.copyDetails': 'Details kopieren',
  'errors.problemActions.startMe': 'Für mich starten',
  'errors.problemActions.makeMyOwnCopy': 'Meine eigene Kopie erstellen (Fork)',
  'errors.problemActions.somethingWentWrongPlease': 'Etwas ist schiefgelaufen. Bitte versuche es erneut.',
  'errors.problemActions.openHostof': '{host} öffnen',
  'errors.problemActions.changeLicence': 'Den Lizenzschlüssel ändern',
  'errors.problemActions.changeSource': 'Ändern, woher die Resources kommen',
  'errors.problemActions.changeVersions': 'Ändern, welchen Versionen gefolgt wird',
  'errors.problemActions.changeMode': 'Ändern, woher die Resources kommen',
  'errors.problemActions.changeName': 'Den Namen oder Ordner ändern',
  'errors.problemActions.changePorts': 'Die Ports ändern',
  'errors.problemActions.changeBuild': 'Den Server-Build ändern',
  'errors.problemActions.changeResources': 'Die Resources ändern',
  'errors.problemActions.changeDatabase': 'Die Datenbank-Wahl ändern',
  'errors.problemActions.changeOptions': 'Die Optionen ändern',
  'errors.problemActions.changeCfx': 'Die txAdmin-Anmeldung ändern',
  'errors.problemActions.changeGithub': 'Das GitHub-Konto ändern',
  'errors.problemActions.thePage': 'die Seite',
} satisfies Record<keyof typeof en, string>

register('de', messages)
