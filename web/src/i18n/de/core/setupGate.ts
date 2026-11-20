import { register } from '../../index'
import type en from '../../en/core/setupGate'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.setupGate.title': 'Einrichten',
  'core.setupGate.intro': 'Kora braucht deinen Cfx.re-Lizenzschlüssel und eine GitHub-Anmeldung, bevor ein Server installiert werden kann. Erledige beides hier, dann folgen die Installationsschritte. Deine anderen Seiten sind nicht betroffen.',
  'core.setupGate.checklist': 'Zuerst zwei Dinge',
  'core.setupGate.stepTitle': '{n}. {title}',
  'core.setupGate.done': 'Erledigt',
  'core.setupGate.todo': 'Noch offen',
  'core.setupGate.cfxTitle': 'Cfx.re-Lizenzschlüssel',
  'core.setupGate.cfxHint': 'Jeder FiveM-Server braucht einen. Schlüssel werden auf diesem Computer gespeichert und bei der Installation per Namen gewählt.',
  'core.setupGate.githubTitle': 'GitHub-Konto',
  'core.setupGate.githubHint': 'Damit holt Kora deine Ressourcen und Forks, auch private.',
} as const satisfies Record<keyof typeof en, string>

register('de', messages)
export default messages
