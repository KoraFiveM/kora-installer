import { register } from '../../index'

const messages = {
  'core.setupGate.title': 'Get set up',
  'core.setupGate.intro': 'Kora needs your Cfx.re licence key and a GitHub sign-in before it can install or manage a server. Finish both here, then carry on.',
  'core.setupGate.checklist': 'Two things first',
  'core.setupGate.stepTitle': '{n}. {title}',
  'core.setupGate.done': 'Done',
  'core.setupGate.todo': 'Still to do',
  'core.setupGate.cfxTitle': 'Cfx.re licence key',
  'core.setupGate.cfxHint': 'Every FiveM server needs one. Keys are saved on this computer and picked by name when you install.',
  'core.setupGate.githubTitle': 'GitHub account',
  'core.setupGate.githubHint': 'Used to fetch your resources and forks, including private ones.',
  'core.setupGate.navShort': 'Set up your Cfx key and GitHub first.',
  'core.setupGate.navReason': 'Add your Cfx.re licence key and sign in to GitHub first. Any page you open shows what is left to do.',
} as const

register('en', messages)
export default messages
