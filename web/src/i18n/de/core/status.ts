import { register } from '../../index'
import type en from '../../en/core/status'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.status.running': 'Läuft',
  'core.status.stopped': 'Angehalten',
  'core.status.folderMissing': 'Ordner fehlt',
  'core.status.notChecked': 'Nicht geprüft',
  'core.status.upDate': 'Aktuell',
  'core.status.needsAttention': 'Braucht Aufmerksamkeit',
  'core.status.newRelease': 'Neue Version',
  'core.status.plainCopiesOriginalRepositorie': 'Einfache Kopien der Original-Repositories',
  'core.status.ownCopiesForksGithub': 'Deine eigenen Kopien (Forks) auf GitHub',
  'core.status.plainCopies': 'Einfache Kopien',
  'core.status.ownCopies': 'Eigene Kopien',
  'core.status.releasedVersions': 'veröffentlichte Versionen',
  'core.status.latestChanges': 'neueste Änderungen',
  'core.status.updates': '{count, plural, one {# Update} other {# Updates}}',
  'core.status.serverRunsContainerIts': 'Dieser Server läuft in einem Container: Seine Dateien liegen in seinem eigenen Ordner, und er startet über die Container-Engine neu.',
  'core.status.inAContainer': 'In einem Container',
} satisfies Record<keyof typeof en, string>

register('de', messages)
