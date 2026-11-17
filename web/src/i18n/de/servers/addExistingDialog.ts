import { register } from '../../index'
import type en from '../../en/servers/addExistingDialog'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'servers.addExistingDialog.addExistingServer': 'Einen vorhandenen Server hinzufügen',
  'servers.addExistingDialog.koraManageFivemServer': 'Kora kann einen FiveM-Server verwalten, den du anders eingerichtet hast. Es ändert deine Dateien nicht.',
  'servers.addExistingDialog.lookServersComputer': 'Nach Servern auf diesem Computer suchen',
  'servers.addExistingDialog.scanServers': 'Nach Servern suchen',
  'servers.addExistingDialog.looking': 'Sucht...',
  'servers.addExistingDialog.noNewServersWere': 'Es wurden keine neuen Server gefunden. Du kannst unten den Ordner eingeben.',
  'servers.addExistingDialog.addName': '{name} hinzufügen',
  'servers.addExistingDialog.add': 'Hinzufügen',
  'servers.addExistingDialog.typeFolder': 'Oder den Ordner eingeben',
  'servers.addExistingDialog.serverFolder': 'Serverordner',
  'servers.addExistingDialog.folderContainsServerCfg': 'Der Ordner, der server.cfg und resources enthält, zum Beispiel /srv/fivem/mein-server.',
  'servers.addExistingDialog.srvFivemMyserver': '/srv/fivem/mein-server',
  'servers.addExistingDialog.chooseServerFolder': 'Wähle deinen Serverordner',
  'servers.addExistingDialog.browse': 'Durchsuchen...',
  'servers.addExistingDialog.addFolder': 'Diesen Ordner hinzufügen',
  'servers.addExistingDialog.addedToList': '{name} wurde zu deiner Liste hinzugefügt.',
} satisfies Record<keyof typeof en, string>

register('de', messages)
