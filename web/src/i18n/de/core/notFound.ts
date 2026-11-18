import { register } from '../../index'
import type en from '../../en/core/notFound'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.notFound.pageFound': 'Seite nicht gefunden',
  'core.notFound.thereNothingAddressMay': 'Unter dieser Adresse gibt es nichts. Sie wurde vielleicht entfernt oder falsch eingegeben.',
  'core.notFound.backServers': 'Zurück zu deinen Servern',
} satisfies Record<keyof typeof en, string>

register('de', messages)
