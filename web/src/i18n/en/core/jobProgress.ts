import { register } from '../../index'

const messages = {
  'core.jobProgress.allStepsFinished': 'All steps are finished.',
  'core.jobProgress.overallProgress': 'Overall progress',
  'core.jobProgress.safetyCopySavedId': 'Safety copy saved ({id}). You can roll back to it.',
  'core.jobProgress.countSizeResourcesFinished': '{count} of {size} resources finished',
  'core.jobProgress.waiting': 'Waiting',
  'core.jobProgress.inProgress': 'In progress',
  'core.jobProgress.done': 'Done',
  'core.jobProgress.skipped': 'Skipped',
  'core.jobProgress.failed': 'Failed',
} as const

register('en', messages)
export default messages
