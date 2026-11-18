import { register } from '../../index'
import type en from '../../en/resources/releasesPanel'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'resources.releasesPanel.releases': 'Releases',
  'resources.releasesPanel.askingGithub': 'GitHub wird gefragt...',
  'resources.releasesPanel.serverPinnedCodeNode': 'Dieser Server ist auf {code, node} festgelegt{newer, plural, =0 {. Das ist das neueste Release.} one {, und es gibt # neueres Release.} other {, und es gibt # neuere Releases.}}',
  'resources.releasesPanel.serverFollowsLatestChanges': 'Dieser Server folgt den neuesten Änderungen, nicht einem Release. Releases von {repo}:',
  'resources.releasesPanel.repoNoReleases': '{repo} hat keine Releases.',
  'resources.releasesPanel.thisServer': 'Dieser Server',
  'resources.releasesPanel.newer': 'Neuer',
  'resources.releasesPanel.prerelease': 'Vorabversion',
  'resources.releasesPanel.openReleaseTagGithub': 'Release {tag} auf GitHub öffnen',
} satisfies Record<keyof typeof en, string>

register('de', messages)
