import { register } from '../../index'

const messages = {
  'servers.versionsCard.whichVersions': 'Which versions',
  'servers.versionsCard.resourcesHereValueSets': 'The resources here are {value}. This sets which versions the server follows.',
  'servers.versionsCard.whichVersionsServerFollows': 'Which versions the server follows',
  'servers.versionsCard.releasedVersionsRecommendedSta': 'Released versions (recommended, stable)',
  'servers.versionsCard.eachResourcePinnedTested': 'Each resource is pinned to a tested release. You choose when to move to a newer one.',
  'servers.versionsCard.latestChangesMainBranch': 'Latest changes (the main branch)',
  'servers.versionsCard.eachResourceFollowsNewest': 'Each resource follows the newest changes, including things that are not released yet.',
  'servers.versionsCard.save': 'Save',
} as const

register('en', messages)
export default messages
