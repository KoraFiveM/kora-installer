import { register } from '../../index'

const messages = {
  'resources.githubTab.githubAccount': 'GitHub account',
  'resources.githubTab.pullRequestsIssuesChanges': 'Pull requests, issues, changes and releases',
  'resources.githubTab.theseLiveEachResources': "These live on each resource's own page: open one from the list above, or from the Resources tab. There you can see what changed, save and push it, open a pull request, try or review other people's, report and comment on issues, and see releases. ",
} as const

register('en', messages)
export default messages
