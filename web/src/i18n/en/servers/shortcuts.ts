import { register } from '../../index'

const messages = {
  'servers.shortcuts.koraNowOpensFrom': 'Kora now opens from your own shortcuts, with no administrator prompt. Tidy up the old ones?',
  'servers.shortcuts.tidyUpOldShortcuts': 'Tidy up old shortcuts',
  'servers.shortcuts.koraShortcuts': 'Kora shortcuts',
  'servers.shortcuts.koraPanelBrowserShortcut': 'Kora is this panel in your browser. The shortcut in your Start Menu (or applications menu) opens it; no administrator prompt.',
  'servers.shortcuts.shortcutPlace': 'Shortcut in place',
  'servers.shortcuts.noShortcutYet': 'No shortcut yet',
  'servers.shortcuts.olderKoraStillInstalled': 'An older Kora is still installed on this computer.',
  'servers.shortcuts.tidyingUpMakesNew': 'Tidying up makes the new shortcuts first, so the old ones keep working until the end. Removing the old copy needs administrator rights; if you are not an administrator here, Kora shows the command to run.',
  'servers.shortcuts.alsoPutShortcutDesktop': 'Also put a shortcut on the desktop (Windows)',
  'servers.shortcuts.refreshShortcuts': 'Refresh the shortcuts',
  'servers.shortcuts.addShortcuts': 'Add the shortcuts',
} as const

register('en', messages)
export default messages
