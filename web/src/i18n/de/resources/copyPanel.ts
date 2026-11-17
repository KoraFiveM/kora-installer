import { register } from '../../index'
import type en from '../../en/resources/copyPanel'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'resources.copyPanel.copyOriginal': ', eine Kopie von {original}',
  'resources.copyPanel.ownCopyNodeNode': 'Deine eigene Kopie: {a, node}{node, node}.',
  'resources.copyPanel.ownCopy': 'Deine eigene Kopie',
  'resources.copyPanel.theOriginal': 'das Original',
  'resources.copyPanel.resourcePlainCopyStrong': 'Diese Resource ist eine einfache Kopie von {strong, node}, deshalb kannst du von ihr keine Änderungen senden oder einen Pull Request öffnen. Wenn du deine eigene Kopie (einen Fork auf GitHub) erstellst, bleibt erhalten, was du hast: dieselbe Version, derselbe Branch und alle Änderungen, die du gemacht hast.',
  'resources.copyPanel.nodeNodeMakeMy': '{node, node}Meine eigene Kopie erstellen (Fork)',
  'resources.copyPanel.doWholeServer': 'Für den ganzen Server tun',
  'resources.copyPanel.someCouldCopied': 'Einige konnten nicht kopiert werden',
  'resources.copyPanel.done': 'Erledigt',
  'resources.copyPanel.serverNowUsesOwn': 'Dieser Server benutzt jetzt deine eigenen Kopien.',
} satisfies Record<keyof typeof en, string>

register('de', messages)
