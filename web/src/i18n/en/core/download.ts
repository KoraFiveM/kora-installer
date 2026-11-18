import { register } from '../../index'

const messages = {
  'core.download.fileTooBigBrowser': 'This file is too big for this browser to download. Copy it from the folder on the computer that runs Kora, or use Chrome or Edge.',
  'core.download.accountMayDownloadBackups': 'Your account may not download backups.',
  'core.download.downloadFailedStatus': 'The download failed ({status}).',
} as const

register('en', messages)
export default messages
