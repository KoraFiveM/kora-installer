import { register } from '../../index'
import type en from '../../en/install/form'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'install.form.typeDatabaseAdministratorUser': 'Gib den Benutzernamen des Datenbank-Administrators ein.',
  'install.form.typeDatabaseAdministratorPassw': 'Gib das Passwort des Datenbank-Administrators ein.',
  'install.form.twoPasswordsSame': 'Die beiden Passwörter stimmen nicht überein.',
  'install.form.databasePortMustNumber': 'Der Datenbank-Port muss eine Zahl von 1 bis 65535 sein.',
  'install.form.typeDatabaseHost': 'Gib den Datenbank-Host ein.',
  'install.form.giveServerName': 'Gib dem Server einen Namen.',
  'install.form.chooseFolderServer': 'Wähle einen Ordner für den Server.',
  'install.form.typeDownloadAddressStarts': 'Gib eine Download-Adresse ein, die mit https:// beginnt.',
  'install.form.pickBuildFromList': 'Wähle einen Build aus der Liste.',
  'install.form.gameBuildMustNumber': 'Der Spiel-Build muss eine Zahl sein.',
  'install.form.pickLeastOneResource': 'Wähle mindestens eine Resource oder entscheide dich, alles zu installieren.',
  'install.form.pickOneSavedKeys': 'Wähle einen deiner gespeicherten Schlüssel oder füge einen neuen hinzu.',
  'install.form.pasteLicenceKeyLong': 'Füge deinen Lizenzschlüssel ein. Er ist eine lange Zeichenfolge aus Buchstaben und Zahlen.',
  'install.form.giveKeyNameFind': 'Gib dem Schlüssel einen Namen, damit du ihn nächstes Mal findest, oder schalte „Diesen Schlüssel speichern“ aus.',
  'install.form.checkCfxReUsername': 'Prüfe den Cfx.re-Benutzernamen oder leere das Feld, um diesen Schritt zu überspringen.',
  'install.form.signGithubContinue': 'Melde dich bei GitHub an, um fortzufahren.',
  'install.form.chooseWhereOwnCopies': 'Wähle, wohin deine eigenen Kopien gehen.',
  'install.form.whereResourcesComeFrom': 'Woher die Resources kommen',
  'install.form.whichVersions': 'Welche Versionen',
  'install.form.nameServer': 'Gib deinem Server einen Namen',
  'install.form.ports': 'Ports',
  'install.form.fivemServerBuild': 'FiveM-Server-Build',
  'install.form.resources': 'Resources',
  'install.form.database': 'Datenbank',
  'install.form.options': 'Optionen',
  'install.form.licenceKey': 'Dein Lizenzschlüssel',
  'install.form.cfxReAccount': 'Dein Cfx.re-Konto',
  'install.form.checkInstall': 'Prüfen und installieren',
  'install.form.checkingPortsFree': 'Es wird geprüft, ob die Ports frei sind...',
  'install.form.onePortsAlreadyUse': 'Einer der Ports wird schon benutzt. Wähle einen freien.',
  'install.form.koraCouldCheckPorts': 'Kora konnte nicht prüfen, ob die Ports frei sind.',
  'install.form.github': 'GitHub',
  'install.form.main': 'Hauptschlüssel',
  'install.form.keyN': 'Schlüssel {n}',
} satisfies Record<keyof typeof en, string>

register('de', messages)
