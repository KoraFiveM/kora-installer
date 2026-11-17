import { register } from '../../index'

const messages = {
  'core.status.running': 'Running',
  'core.status.stopped': 'Stopped',
  'core.status.folderMissing': 'Folder missing',
  'core.status.notChecked': 'Not checked',
  'core.status.upDate': 'Up to date',
  'core.status.needsAttention': 'Needs attention',
  'core.status.newRelease': 'New release',
  'core.status.plainCopiesOriginalRepositorie': 'Plain copies of the original repositories',
  'core.status.ownCopiesForksGithub': 'Your own copies (forks) on GitHub',
  'core.status.plainCopies': 'Plain copies',
  'core.status.ownCopies': 'Own copies',
  'core.status.releasedVersions': 'released versions',
  'core.status.latestChanges': 'latest changes',
  'core.status.updates': '{count, plural, one {# update} other {# updates}}',
} as const

register('en', messages)
export default messages
