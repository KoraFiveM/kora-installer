import { register } from '../../index'
import type en from '../../en/settings/changeLicenceKey'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'settings.changeLicenceKey.licenceKey': 'Lizenzschlüssel',
  'settings.changeLicenceKey.serverUsesSavedLicence': 'Dieser Server nutzt deinen gespeicherten Lizenzschlüssel {strong, node} (endet auf {last4}).',
  'settings.changeLicenceKey.licenceKeyPartServers': 'Der Lizenzschlüssel ist Teil der eigenen Konfiguration dieses Servers.',
  'settings.changeLicenceKey.nodeNodeChangeHere': '{node, node} Ändere ihn hier, wenn du einen neuen Schlüssel bekommen hast oder diesen Server auf einen anderen umstellen willst. ',
  'settings.changeLicenceKey.changeLicenceKey': 'Lizenzschlüssel ändern',
  'settings.changeLicenceKey.licenceKeyChangedNow': 'Lizenzschlüssel geändert (endet jetzt auf {last4})',
  'settings.changeLicenceKey.restartServerNow': 'Den Server jetzt neu starten',
  'settings.changeLicenceKey.pickOneSavedLicence': 'Wähle einen der gespeicherten Lizenzschlüssel.',
  'settings.changeLicenceKey.pasteNewLicenceKey': 'Füge den neuen Lizenzschlüssel ein.',
  'settings.changeLicenceKey.giveKeyNameTurn': 'Gib dem Schlüssel einen Namen oder schalte „Diesen Schlüssel speichern“ aus.',
  'settings.changeLicenceKey.chooseKeyNameShould': 'Wähle den Schlüssel, den {name} verwenden soll. Kora schreibt ihn in die eigene Konfiguration des Servers.',
  'settings.changeLicenceKey.koraCannotReadSaved': 'Kora kann deine gespeicherten Lizenzschlüssel nicht lesen',
  'settings.changeLicenceKey.savedKeys': 'Gespeicherte Schlüssel',
  'settings.changeLicenceKey.serverUsesNow': '. Dieser Server nutzt ihn jetzt.',
  'settings.changeLicenceKey.keyEndingLastValue': 'Schlüssel mit Endung {last4}{value}',
  'settings.changeLicenceKey.newKey': 'Ein neuer Schlüssel',
  'settings.changeLicenceKey.pasteKeySavedYet': 'Füge einen Schlüssel ein, den du noch nicht gespeichert hast.',
  'settings.changeLicenceKey.thereNoSavedLicence': 'Es gibt keine gespeicherten Lizenzschlüssel zur Auswahl. Bitte einen Administrator, in den Einstellungen einen hinzuzufügen.',
  'settings.changeLicenceKey.newLicenceKey': 'Neuer Lizenzschlüssel',
  'settings.changeLicenceKey.getOnePortalCfx': 'Hole dir einen auf portal.cfx.re unter Server keys.',
  'settings.changeLicenceKey.saveKeyComputer': 'Diesen Schlüssel auf diesem Computer speichern',
  'settings.changeLicenceKey.pickNameNextTime': 'Damit du ihn nächstes Mal mit seinem Namen wählen kannst.',
  'settings.changeLicenceKey.nameKey': 'Name für diesen Schlüssel',
  'settings.changeLicenceKey.serverRunningOnlyReads': 'Dieser Server läuft. Er liest seinen Schlüssel nur beim Start, deshalb wirst du nach der Änderung gebeten, ihn neu zu starten.',
  'settings.changeLicenceKey.serverStoppedNoRestart': 'Dieser Server ist angehalten, es ist also kein Neustart nötig: Er nutzt den neuen Schlüssel beim nächsten Start.',
  'settings.changeLicenceKey.cancel': 'Abbrechen',
  'settings.changeLicenceKey.defaultKeyName': 'Haupt',
} satisfies Record<keyof typeof en, string>

register('de', messages)
