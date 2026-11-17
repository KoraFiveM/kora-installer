import { register } from '../../index'

const messages = {
  'resources.branchesPanel.branchesPullRequests': 'Branches and pull requests',
  'resources.branchesPanel.eachResourceServerOwn': 'Each resource on this server is your own copy of it. Here you can see which version of the code each one runs, try a change someone has proposed (a pull request), or go back to the normal version.',
  'resources.branchesPanel.lookingResources': 'Looking at your resources...',
  'resources.branchesPanel.countResourceValueNormal': '{count, plural, one {# resource is not on the normal version} other {# resources are not on the normal version}}',
  'resources.branchesPanel.updatesLeaveValueAlone': '{count, plural, one {Updates leave it alone until it goes back, so nothing is pulled into your branch by accident.} other {Updates leave them alone until they go back, so nothing is pulled into your branch by accident.}} ',
  'resources.branchesPanel.noneServersResourcesChange': "None of this server's resources can change branch.",
  'resources.branchesPanel.resourcesTheirBranches': 'Resources and their branches',
  'resources.branchesPanel.openPageId': 'Open the page of {id}',
  'resources.branchesPanel.changedCountChangedFile': '{changed_count, plural, one {# changed file} other {# changed files}}',
  'resources.branchesPanel.stashesKeptAside': '{stashes} kept aside',
  'resources.branchesPanel.changeBranchId': 'Change the branch of {id}',
  'resources.branchesPanel.change': 'Change…',
  'resources.branchesPanel.severalResourcesOnce': 'Several resources at once',
} as const

register('en', messages)
export default messages
