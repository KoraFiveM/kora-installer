import { register } from '../../index'

const messages = {
  'settings.changeLicenceKey.licenceKey': 'Licence key',
  'settings.changeLicenceKey.serverUsesSavedLicence': 'This server uses your saved licence key {strong, node} (ending {last4}).',
  'settings.changeLicenceKey.licenceKeyPartServers': 'The licence key is part of this server’s own configuration.',
  'settings.changeLicenceKey.nodeNodeChangeHere': '{node, node} Change it here if you got a new key or want to move this server to another one. ',
  'settings.changeLicenceKey.changeLicenceKey': 'Change licence key',
  'settings.changeLicenceKey.licenceKeyChangedNow': 'Licence key changed (now ending {last4})',
  'settings.changeLicenceKey.restartServerNow': 'Restart the server now',
  'settings.changeLicenceKey.pickOneSavedLicence': 'Pick one of the saved licence keys.',
  'settings.changeLicenceKey.pasteNewLicenceKey': 'Paste the new licence key.',
  'settings.changeLicenceKey.giveKeyNameTurn': 'Give the key a name, or turn off "Save this key".',
  'settings.changeLicenceKey.chooseKeyNameShould': "Choose the key {name} should use. Kora writes it into the server's own configuration.",
  'settings.changeLicenceKey.koraCannotReadSaved': 'Kora cannot read your saved licence keys',
  'settings.changeLicenceKey.savedKeys': 'Saved keys',
  'settings.changeLicenceKey.serverUsesNow': '. This server uses it now.',
  'settings.changeLicenceKey.keyEndingLastValue': 'Key ending {last4}{value}',
  'settings.changeLicenceKey.newKey': 'A new key',
  'settings.changeLicenceKey.pasteKeySavedYet': 'Paste a key you have not saved yet.',
  'settings.changeLicenceKey.thereNoSavedLicence': 'There are no saved licence keys to pick from. Ask an administrator to add one in Settings.',
  'settings.changeLicenceKey.newLicenceKey': 'New licence key',
  'settings.changeLicenceKey.getOnePortalCfx': 'Get one at portal.cfx.re, under Server keys.',
  'settings.changeLicenceKey.saveKeyComputer': 'Save this key on this computer',
  'settings.changeLicenceKey.pickNameNextTime': 'So you can pick it by name next time.',
  'settings.changeLicenceKey.nameKey': 'Name for this key',
  'settings.changeLicenceKey.serverRunningOnlyReads': 'This server is running. It only reads its key when it starts, so you will be asked to restart it after the change.',
  'settings.changeLicenceKey.serverStoppedNoRestart': 'This server is stopped, so no restart is needed: it uses the new key next time it starts.',
  'settings.changeLicenceKey.cancel': 'Cancel',
  'settings.changeLicenceKey.defaultKeyName': 'Main',
} as const

register('en', messages)
export default messages
