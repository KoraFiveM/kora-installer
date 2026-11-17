import { register } from '../../index'

const messages = {
  'resources.issuesPanel.issues': 'Issues',
  'resources.issuesPanel.askingGithub': 'Asking GitHub...',
  'resources.issuesPanel.thereNoOpenIssues': 'There are no open issues on {repo}.',
  'resources.issuesPanel.openIssues': 'Open issues',
  'resources.issuesPanel.authorCommentsCommentValue': 'by {author}, {comments, plural, one {# comment} other {# comments}}',
  'resources.issuesPanel.openIssueNumberGithub': 'Open issue #{number} on GitHub',
  'resources.issuesPanel.comment': 'Comment',
  'resources.issuesPanel.commentIssueNumber': 'Comment on issue #{number}',
  'resources.issuesPanel.sendComment': 'Send comment',
  'resources.issuesPanel.reportNewIssue': 'Report a new issue',
  'resources.issuesPanel.title': 'Title',
  'resources.issuesPanel.whatHappened': 'What happened?',
  'resources.issuesPanel.koraAddsItsVersion': "Kora adds its version and the resource's version. It never adds secrets or file paths.",
  'resources.issuesPanel.addShortSummaryComputer': 'Add a short summary of this computer',
  'resources.issuesPanel.koraVersionSystemTool': 'Your Kora version, system and tool versions, with secrets removed. The support bundle file is not attached. Issues are public.',
  'resources.issuesPanel.showWhatAdded': 'Show what will be added',
  'resources.issuesPanel.summaryAdded': 'Summary that will be added',
  'resources.issuesPanel.reportIssue': 'Report issue',
  'resources.issuesPanel.issueReported': 'Issue reported',
} as const

register('en', messages)
export default messages
