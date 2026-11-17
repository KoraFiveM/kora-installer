import { register } from '../../index'
import type en from '../../en/resources/openPullRequest'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'resources.openPullRequest.openPullRequest': 'Einen Pull Request öffnen',
  'resources.openPullRequest.shareChangesOriginalAuthor': 'Deine Änderungen mit dem ursprünglichen Autor teilen',
  'resources.openPullRequest.pullRequestAsksAuthor': 'Ein Pull Request bittet den Autor, deine Änderungen zu übernehmen. Kora prüft vorher, ob du Commits hast, die er nicht hat.',
  'resources.openPullRequest.checkMyChanges': 'Meine Änderungen prüfen',
  'resources.openPullRequest.readySend': 'Bereit zum Senden',
  'resources.openPullRequest.aheadChangeValueFrom': '{ahead, plural, one {# Änderung} other {# Änderungen}} von {head_owner}:{head_branch} nach {upstream} ({base}).',
  'resources.openPullRequest.title': 'Titel',
  'resources.openPullRequest.description': 'Beschreibung',
  'resources.openPullRequest.openPullRequest2': 'Pull Request öffnen',
  'resources.openPullRequest.pullRequestOpened': 'Pull Request geöffnet',
} satisfies Record<keyof typeof en, string>

register('de', messages)
