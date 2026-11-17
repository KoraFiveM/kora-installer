import { register } from '../../index'
import type en from '../../en/resources/issuesPanel'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'resources.issuesPanel.issues': 'Issues',
  'resources.issuesPanel.askingGithub': 'GitHub wird gefragt...',
  'resources.issuesPanel.thereNoOpenIssues': 'In {repo} gibt es keine offenen Issues.',
  'resources.issuesPanel.openIssues': 'Offene Issues',
  'resources.issuesPanel.authorCommentsCommentValue': 'von {author}, {comments, plural, one {# Kommentar} other {# Kommentare}}',
  'resources.issuesPanel.openIssueNumberGithub': 'Issue #{number} auf GitHub öffnen',
  'resources.issuesPanel.comment': 'Kommentar',
  'resources.issuesPanel.commentIssueNumber': 'Issue #{number} kommentieren',
  'resources.issuesPanel.sendComment': 'Kommentar senden',
  'resources.issuesPanel.reportNewIssue': 'Neues Issue melden',
  'resources.issuesPanel.title': 'Titel',
  'resources.issuesPanel.whatHappened': 'Was ist passiert?',
  'resources.issuesPanel.koraAddsItsVersion': 'Kora fügt seine Version und die Version der Resource hinzu. Es fügt nie Geheimnisse oder Dateipfade hinzu.',
  'resources.issuesPanel.addShortSummaryComputer': 'Eine kurze Zusammenfassung dieses Computers hinzufügen',
  'resources.issuesPanel.koraVersionSystemTool': 'Deine Kora-Version, dein System und Werkzeugversionen, ohne Geheimnisse. Die Support-Paket-Datei wird nicht angehängt. Issues sind öffentlich.',
  'resources.issuesPanel.showWhatAdded': 'Anzeigen, was hinzugefügt wird',
  'resources.issuesPanel.summaryAdded': 'Zusammenfassung, die hinzugefügt wird',
  'resources.issuesPanel.reportIssue': 'Issue melden',
  'resources.issuesPanel.issueReported': 'Issue gemeldet',
} satisfies Record<keyof typeof en, string>

register('de', messages)
