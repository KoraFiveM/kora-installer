import { register } from '../../index'

const messages = {
  'core.jobHistory.openInstallCarry': 'Open Install to carry on',
  'core.jobHistory.recentJobs': 'Recent jobs',
  'core.jobHistory.updatesInstallsOtherJobs': 'Updates, installs and other jobs you started. They are kept when Kora Panel restarts. A job that was still running when the panel stopped shows as stopped by a restart, with what to do next.',
  'core.jobHistory.loading': 'Loading...',
  'core.jobHistory.nothingRunYet': 'Nothing has run yet.',
  'core.jobHistory.running': 'Running',
  'core.jobHistory.finished': 'Finished',
  'core.jobHistory.didFinish': 'Did not finish',
  'core.jobHistory.stoppedRestart': 'Stopped by a restart',
} as const

register('en', messages)
export default messages
