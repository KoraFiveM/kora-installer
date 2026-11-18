import { register } from '../../index'
import type en from '../../en/core/auditLog'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.auditLog.done': 'Erledigt',
  'core.auditLog.started': 'Gestartet',
  'core.auditLog.failed': 'Fehlgeschlagen',
  'core.auditLog.notAllowed': 'Nicht erlaubt',
} satisfies Record<keyof typeof en, string>

register('de', messages)
