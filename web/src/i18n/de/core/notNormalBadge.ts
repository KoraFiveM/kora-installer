import { register } from '../../index'
import type en from '../../en/core/notNormalBadge'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.notNormalBadge.tryingPullRequestPr': 'Testet Pull Request #{pr}',
  'core.notNormalBadge.singleCommit': 'Auf einem einzelnen Commit',
  'core.notNormalBadge.branchBranch': 'Auf Branch {branch}',
  'core.notNormalBadge.itsNormalVersionDefault': 'Nicht auf der normalen Version ({default_branch}). {text}.',
  'core.notNormalBadge.itsNormalVersion': 'Nicht auf der normalen Version: ',
  'core.notNormalBadge.tryingPullRequestTitle': 'Testet Pull Request #{pr}: {title}',
  'core.notNormalBadge.summary': '{count, plural, one {# Resource nicht auf der normalen Version} other {# Resources nicht auf der normalen Version}}',
} satisfies Record<keyof typeof en, string>

register('de', messages)
