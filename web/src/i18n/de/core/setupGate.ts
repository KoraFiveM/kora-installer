import { register } from '../../index'
import type en from '../../en/core/setupGate'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.setupGate.title': 'Einrichten',
  'core.setupGate.intro': 'Kora braucht deinen Cfx.re-Lizenzschlüssel und eine GitHub-Anmeldung, bevor ein Server installiert oder verwaltet werden kann. Erledige beides hier und mach dann weiter.',
  'core.setupGate.checklist': 'Zuerst zwei Dinge',
  'core.setupGate.stepTitle': '{n}. {title}',
  'core.setupGate.done': 'Erledigt',
  'core.setupGate.todo': 'Noch offen',
  'core.setupGate.cfxTitle': 'Cfx.re-Lizenzschlüssel',
  'core.setupGate.cfxHint': 'Jeder FiveM-Server braucht einen. Schlüssel werden auf diesem Computer gespeichert und bei der Installation per Namen gewählt.',
  'core.setupGate.githubTitle': 'GitHub-Konto',
  'core.setupGate.githubHint': 'Damit holt Kora deine Ressourcen und Forks, auch private.',
  'core.setupGate.navShort': 'Richte zuerst Cfx-Schlüssel und GitHub ein.',
  'core.setupGate.navReason': 'Füge zuerst deinen Cfx.re-Lizenzschlüssel hinzu und melde dich bei GitHub an. Jede Seite, die du öffnest, zeigt, was noch fehlt.',
} as const satisfies Record<keyof typeof en, string>

register('de', messages)
export default messages
