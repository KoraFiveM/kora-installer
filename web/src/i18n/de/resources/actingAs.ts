import { register } from '../../index'
import type en from '../../en/resources/actingAs'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'resources.actingAs.doneGithubStrongNode': 'Das geschieht auf GitHub als {strong, node}, dem Konto, mit dem dieser Computer angemeldet ist. Es wird von allen geteilt, die dieses Panel nutzen.',
  'resources.actingAs.doneGithubAccountComputer': 'Das geschieht als das GitHub-Konto, mit dem dieser Computer angemeldet ist, geteilt von allen, die dieses Panel nutzen.',
} satisfies Record<keyof typeof en, string>

register('de', messages)
