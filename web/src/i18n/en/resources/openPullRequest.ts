import { register } from '../../index'

const messages = {
  'resources.openPullRequest.openPullRequest': 'Open a pull request',
  'resources.openPullRequest.shareChangesOriginalAuthor': 'Share your changes with the original author',
  'resources.openPullRequest.pullRequestAsksAuthor': 'A pull request asks the author to take your changes. Kora checks first that you have commits they do not.',
  'resources.openPullRequest.checkMyChanges': 'Check my changes',
  'resources.openPullRequest.readySend': 'Ready to send',
  'resources.openPullRequest.aheadChangeValueFrom': '{ahead, plural, one {# change} other {# changes}} from {head_owner}:{head_branch} to {upstream} ({base}).',
  'resources.openPullRequest.title': 'Title',
  'resources.openPullRequest.description': 'Description',
  'resources.openPullRequest.openPullRequest2': 'Open pull request',
  'resources.openPullRequest.pullRequestOpened': 'Pull request opened',
} as const

register('en', messages)
export default messages
