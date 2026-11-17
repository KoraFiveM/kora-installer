import { register } from '../../index'
import type en from '../../en/core/store'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.store.nSignedFromAnother': '{n} (von einem anderen Gerät angemeldet{kind, select, both { mit {provider}, {role}} provider { mit {provider}} role {, {role}} other {}})',
  'core.store.nKoraCommand': '{n} (ein kora-Befehl)',
  'core.store.nComputer': '{n} (auf diesem Computer)',
  'core.store.someone': 'jemand',
} satisfies Record<keyof typeof en, string>

register('de', messages)
