import { register } from '../../index'
import type en from '../../en/resources/restartNotice'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'resources.restartNotice.whatChangedWhy': 'Was sich geändert hat: {why}.',
  'resources.restartNotice.nodeNodeRestartResource': '{node, node} Diese Resource neu starten ',
  'resources.restartNotice.restartWholeServer': 'Den ganzen Server neu starten',
  'resources.restartNotice.restartNeededRunningServer': 'Neustart nötig: Der laufende Server hat noch die alte Version.',
} satisfies Record<keyof typeof en, string>

register('de', messages)
