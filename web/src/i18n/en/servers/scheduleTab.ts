import { register } from '../../index'

const messages = {
  'servers.scheduleTab.automaticUpdatesOff': 'Automatic updates are off.',
  'servers.scheduleTab.savedServerUpdatesItself': 'Saved. The server updates itself every {day} at {time}.',
  'servers.scheduleTab.automaticUpdates': 'Automatic updates',
  'servers.scheduleTab.everyServerUpdateItself': 'Every server can update itself on a schedule. Kora checks for newer versions (released versions or the latest changes, whichever the server follows), updates what is safe to update and checks the server still starts. It is off until you turn it on.',
  'servers.scheduleTab.currentSchedule': 'Current schedule',
  'servers.scheduleTab.howOften': 'How often',
  'servers.scheduleTab.off': 'Off',
  'servers.scheduleTab.updatesOnlyHappenWhen': 'Updates only happen when you run them yourself.',
  'servers.scheduleTab.everyDay': 'Every day',
  'servers.scheduleTab.onceWeek': 'Once a week',
  'servers.scheduleTab.day': 'Day',
  'servers.scheduleTab.timeHours': 'Time (24 hours)',
  'servers.scheduleTab.useFormHhMm': 'Use the form HH:MM, for example 04:00.',
  'servers.scheduleTab.runUpdate': 'Run the update as',
  'servers.scheduleTab.you': 'You',
  'servers.scheduleTab.usesGithubSigninPrivate': 'Uses your GitHub sign-in, so private forks can be fetched. Runs while your account can run background tasks.',
  'servers.scheduleTab.systemAccount': 'The system account',
  'servers.scheduleTab.runsEvenWhenNobody': 'Runs even when nobody is signed in, but has no GitHub sign-in, so private forks cannot be fetched. Needs administrator rights to set up.',
  'servers.scheduleTab.turnScheduleOff': 'Turn schedule off',
  'servers.scheduleTab.saveSchedule': 'Save schedule',
} as const

register('en', messages)
export default messages
