import { register } from '../../index'
import type en from '../../en/core/client'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.client.serverAnsweredUnexpectedError': 'Der Server hat mit einem unerwarteten Fehler geantwortet ({status}).',
  'core.client.couldReachKoraCheck': 'Kora war nicht erreichbar. Prüfe, ob es noch läuft, und versuche es dann erneut.',
} satisfies Record<keyof typeof en, string>

register('de', messages)
