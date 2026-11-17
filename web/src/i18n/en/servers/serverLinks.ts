import { register } from '../../index'

const messages = {
  'servers.serverLinks.copyPath': 'Copy path',
  'servers.serverLinks.couldOpenHereFailed': 'Could not open it here: {failed}',
  'servers.serverLinks.folderopenNodeOpenFolder': '{folderOpen, node} Open folder ',
  'servers.serverLinks.txadminAnswersOnceServer': 'txAdmin answers once the server is running.',
  'servers.serverLinks.externallinkNodeOpenTxadmin': '{externalLink, node} Open txAdmin',
  'servers.serverLinks.browserBlockedNewTab': 'Your browser blocked the new tab: click here to open txAdmin.',
  'servers.serverLinks.secretsTxadminadminTxt': 'secrets/txadmin-admin.txt',
  'servers.serverLinks.useAccountStrongNode': ': use the account {strong, node} that Kora made when it installed the server (its password is in <code>secrets/txadmin-admin.txt</code> in the server folder)',
  'servers.serverLinks.txadminItsOwnSignin': 'txAdmin has its own sign-in{node, node}. The panel cannot sign you in to it.',
  'servers.serverLinks.copyConnectAddress': 'Copy connect address',
  'servers.serverLinks.unknownError': 'unknown error',
} as const

register('en', messages)
export default messages
