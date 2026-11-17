import { register } from '../../index'
import type en from '../../en/servers/serverLinks'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'servers.serverLinks.copyPath': 'Pfad kopieren',
  'servers.serverLinks.couldOpenHereFailed': 'Konnte hier nicht geöffnet werden: {failed}',
  'servers.serverLinks.folderopenNodeOpenFolder': '{folderOpen, node} Ordner öffnen ',
  'servers.serverLinks.txadminAnswersOnceServer': 'txAdmin antwortet, sobald der Server läuft.',
  'servers.serverLinks.externallinkNodeOpenTxadmin': '{externalLink, node} txAdmin öffnen',
  'servers.serverLinks.browserBlockedNewTab': 'Dein Browser hat den neuen Tab blockiert: Klicke hier, um txAdmin zu öffnen.',
  'servers.serverLinks.useAccountStrongNode': ': Nutze das Konto {strong, node}, das Kora bei der Installation des Servers erstellt hat (sein Passwort steht in <code>secrets/txadmin-admin.txt</code> im Serverordner)',
  'servers.serverLinks.txadminItsOwnSignin': 'txAdmin hat seine eigene Anmeldung{node, node}. Das Panel kann dich dort nicht anmelden.',
  'servers.serverLinks.copyConnectAddress': 'Verbindungsadresse kopieren',
  'servers.serverLinks.unknownError': 'unbekannter Fehler',
} satisfies Record<keyof typeof en, string>

register('de', messages)
