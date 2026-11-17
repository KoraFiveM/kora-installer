import { register } from '../../index'

const messages = {
  'resources.releasesPanel.releases': 'Releases',
  'resources.releasesPanel.askingGithub': 'Asking GitHub...',
  'resources.releasesPanel.serverPinnedCodeNode': 'This server is pinned to {code, node}{newer, plural, =0 {. It is the newest release.} one {, and # newer release exists.} other {, and # newer releases exist.}}',
  'resources.releasesPanel.serverFollowsLatestChanges': 'This server follows the latest changes, not a release. Releases of {repo}:',
  'resources.releasesPanel.repoNoReleases': '{repo} has no releases.',
  'resources.releasesPanel.thisServer': 'This server',
  'resources.releasesPanel.newer': 'Newer',
  'resources.releasesPanel.prerelease': 'Pre-release',
  'resources.releasesPanel.openReleaseTagGithub': 'Open release {tag} on GitHub',
} as const

register('en', messages)
export default messages
