import { register } from '../../index'
import type en from '../../en/resources/pullRequestsPanel'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'resources.pullRequestsPanel.noChecks': 'Keine Checks',
  'resources.pullRequestsPanel.checkcircleNodeChecksPassing': '{checkCircle2, node}Checks bestanden',
  'resources.pullRequestsPanel.xcircleNodeChecksFailing': '{xCircle, node}Checks fehlgeschlagen',
  'resources.pullRequestsPanel.circledashedNodeChecksRunning': '{circleDashed, node}Checks laufen',
  'resources.pullRequestsPanel.approved': 'Genehmigt',
  'resources.pullRequestsPanel.changesRequested': 'Änderungen angefordert',
  'resources.pullRequestsPanel.waitingReview': 'Wartet auf eine Review',
  'resources.pullRequestsPanel.pullRequests': 'Pull Requests',
  'resources.pullRequestsPanel.pullRequestChangeSomeone': 'Ein Pull Request ist eine Änderung, die jemand vorgeschlagen hat. Kora kann einen herunterladen, damit du ihn auf diesem Server ausprobieren kannst, bevor er akzeptiert wird.',
  'resources.pullRequestsPanel.pullRequestBeingTried': 'Der Pull Request, der gerade getestet wird',
  'resources.pullRequestsPanel.tryingPullRequestTrying': 'Du testest Pull Request #{trying}',
  'resources.pullRequestsPanel.nodeNodeGetLatest': '{node, node}Neueste Commits holen',
  'resources.pullRequestsPanel.stopTrying': 'Nicht mehr testen',
  'resources.pullRequestsPanel.runningServerStillOld': 'Der laufende Server hat bis zu seinem Neustart noch die alte Version. Den Knopf dafür findest du im Hinweis oben auf dieser Seite.',
  'resources.pullRequestsPanel.pullRequestSomeoneElses': 'Ein Pull Request ist der Code eines anderen. Sobald du ihn ausprobierst, läuft er auf deinem Server wie jede seiner Resources, bevor ihn jemand akzeptiert hat. Probiere nur Pull Requests von Leuten aus, denen du vertraust, und sieh dir vorher auf GitHub an, was sie ändern.',
  'resources.pullRequestsPanel.serverFollowsReleasedVersions': 'Dieser Server folgt veröffentlichten Versionen',
  'resources.pullRequestsPanel.serverMeantRunOnly': 'Dieser Server soll nur veröffentlichte Versionen ausführen. Wenn du einen Pull Request testest, führt diese Resource unveröffentlichten Code aus, bis du das Testen beendest.',
  'resources.pullRequestsPanel.iUnderstandResourceRun': 'Ich verstehe: Diese Resource führt unveröffentlichten Code aus',
  'resources.pullRequestsPanel.searchPullRequests': 'Pull Requests durchsuchen',
  'resources.pullRequestsPanel.searchNumberTitleAuthor': 'Nach Nummer, Titel oder Autor suchen',
  'resources.pullRequestsPanel.askingGithub': 'GitHub wird gefragt...',
  'resources.pullRequestsPanel.noPullRequestMatches': 'Kein Pull Request passt zu deiner Suche.',
  'resources.pullRequestsPanel.thereNoOpenPull': 'Es gibt keine offenen Pull Requests.',
  'resources.pullRequestsPanel.openPullRequests': 'Offene Pull Requests',
  'resources.pullRequestsPanel.draft': 'Entwurf',
  'resources.pullRequestsPanel.byAuthor': 'von {author}',
  'resources.pullRequestsPanel.openedValueUpdatedValue': 'geöffnet {opened}, aktualisiert {updated}',
  'resources.pullRequestsPanel.itsForkDeleted': 'sein Fork wurde gelöscht',
  'resources.pullRequestsPanel.openPullRequestNumber': 'Pull Request #{number} auf GitHub öffnen',
  'resources.pullRequestsPanel.hideDetails': 'Details ausblenden',
  'resources.pullRequestsPanel.details': 'Details',
  'resources.pullRequestsPanel.tickBoxAboveFirst': 'Hake zuerst das Kästchen oben an',
  'resources.pullRequestsPanel.tryPullRequestNumber': 'Ausprobieren: Pull Request #{number}',
  'resources.pullRequestsPanel.tryIt': 'Ausprobieren',
  'resources.pullRequestsPanel.originalRepository': 'Ursprüngliches Repository',
  'resources.pullRequestsPanel.ownCopy': 'Deine eigene Kopie',
} satisfies Record<keyof typeof en, string>

register('de', messages)
