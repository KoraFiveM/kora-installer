import { register } from '../../index'

const messages = {
  'core.notNormalBadge.tryingPullRequestPr': 'Trying pull request #{pr}',
  'core.notNormalBadge.singleCommit': 'On a single commit',
  'core.notNormalBadge.branchBranch': 'On branch {branch}',
  'core.notNormalBadge.itsNormalVersionDefault': 'Not on its normal version ({default_branch}). {text}.',
  'core.notNormalBadge.itsNormalVersion': 'Not on its normal version: ',
  'core.notNormalBadge.tryingPullRequestTitle': 'Trying pull request #{pr}: {title}',
  'core.notNormalBadge.summary': '{count, plural, one {# resource not on its normal version} other {# resources not on their normal version}}',
} as const

register('en', messages)
export default messages
