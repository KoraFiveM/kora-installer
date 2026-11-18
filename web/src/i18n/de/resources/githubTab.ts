import { register } from '../../index'
import type en from '../../en/resources/githubTab'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'resources.githubTab.githubAccount': 'GitHub-Konto',
  'resources.githubTab.pullRequestsIssuesChanges': 'Pull Requests, Issues, Änderungen und Releases',
  'resources.githubTab.theseLiveEachResources': 'Diese findest du auf der eigenen Seite jeder Resource: Öffne eine aus der Liste oben oder aus dem Tab Resources. Dort siehst du, was sich geändert hat, kannst es speichern und pushen, einen Pull Request öffnen, die anderer ausprobieren oder prüfen, Issues melden und kommentieren und Releases ansehen. ',
} satisfies Record<keyof typeof en, string>

register('de', messages)
