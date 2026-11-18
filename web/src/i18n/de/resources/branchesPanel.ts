import { register } from '../../index'
import type en from '../../en/resources/branchesPanel'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'resources.branchesPanel.branchesPullRequests': 'Branches und Pull Requests',
  'resources.branchesPanel.eachResourceServerOwn': 'Jede Resource auf diesem Server ist deine eigene Kopie davon. Hier siehst du, welche Version des Codes jede ausführt, kannst eine vorgeschlagene Änderung (einen Pull Request) ausprobieren oder zur normalen Version zurückgehen.',
  'resources.branchesPanel.lookingResources': 'Deine Resources werden angesehen...',
  'resources.branchesPanel.countResourceValueNormal': '{count, plural, one {# Resource ist nicht auf der normalen Version} other {# Resources sind nicht auf der normalen Version}}',
  'resources.branchesPanel.updatesLeaveValueAlone': '{count, plural, one {Updates lassen sie in Ruhe, bis sie zurückgeht, damit nichts versehentlich in deinen Branch gezogen wird.} other {Updates lassen sie in Ruhe, bis sie zurückgehen, damit nichts versehentlich in deinen Branch gezogen wird.}} ',
  'resources.branchesPanel.noneServersResourcesChange': 'Keine der Resources dieses Servers kann den Branch wechseln.',
  'resources.branchesPanel.resourcesTheirBranches': 'Resources und ihre Branches',
  'resources.branchesPanel.openPageId': 'Seite von {id} öffnen',
  'resources.branchesPanel.changedCountChangedFile': '{changed_count, plural, one {# geänderte Datei} other {# geänderte Dateien}}',
  'resources.branchesPanel.stashesKeptAside': '{stashes} beiseitegelegt',
  'resources.branchesPanel.changeBranchId': 'Branch von {id} ändern',
  'resources.branchesPanel.change': 'Ändern…',
  'resources.branchesPanel.severalResourcesOnce': 'Mehrere Resources auf einmal',
} satisfies Record<keyof typeof en, string>

register('de', messages)
