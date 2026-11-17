import { register } from '../../index'
import type en from '../../en/core/download'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.download.fileTooBigBrowser': 'Diese Datei ist zu groß, als dass dieser Browser sie herunterladen könnte. Kopiere sie aus dem Ordner auf dem Computer, auf dem Kora läuft, oder nutze Chrome oder Edge.',
  'core.download.accountMayDownloadBackups': 'Dein Konto darf keine Backups herunterladen.',
  'core.download.downloadFailedStatus': 'Der Download ist fehlgeschlagen ({status}).',
} satisfies Record<keyof typeof en, string>

register('de', messages)
