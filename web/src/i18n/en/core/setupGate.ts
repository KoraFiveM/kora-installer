import { register } from '../../index'

const messages = {
  'core.setupGate.title': 'Get set up',
  'core.setupGate.intro': 'Kora needs your Cfx.re licence key and a GitHub sign-in before it can install a server. Finish both here and the install steps follow. Your other pages are not affected.',
  'core.setupGate.checklist': 'Two things first',
  'core.setupGate.stepTitle': '{n}. {title}',
  'core.setupGate.done': 'Done',
  'core.setupGate.todo': 'Still to do',
  'core.setupGate.cfxTitle': 'Cfx.re licence key',
  'core.setupGate.cfxHint': 'Every FiveM server needs one. Keys are saved on this computer and picked by name when you install.',
  'core.setupGate.githubTitle': 'GitHub account',
  'core.setupGate.githubHint': 'Used to fetch your resources and forks, including private ones.',
} as const

register('en', messages)
export default messages
