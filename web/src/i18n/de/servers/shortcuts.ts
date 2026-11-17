import { register } from '../../index'
import type en from '../../en/servers/shortcuts'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'servers.shortcuts.koraNowOpensFrom': 'Kora öffnet sich jetzt über deine eigenen Verknüpfungen, ohne Administrator-Abfrage. Die alten aufräumen?',
  'servers.shortcuts.tidyUpOldShortcuts': 'Alte Verknüpfungen aufräumen',
  'servers.shortcuts.koraShortcuts': 'Kora-Verknüpfungen',
  'servers.shortcuts.koraPanelBrowserShortcut': 'Kora ist dieses Panel in deinem Browser. Die Verknüpfung in deinem Startmenü (oder Anwendungsmenü) öffnet es; keine Administrator-Abfrage.',
  'servers.shortcuts.shortcutPlace': 'Verknüpfung vorhanden',
  'servers.shortcuts.noShortcutYet': 'Noch keine Verknüpfung',
  'servers.shortcuts.olderKoraStillInstalled': 'Auf diesem Computer ist noch ein älteres Kora installiert.',
  'servers.shortcuts.tidyingUpMakesNew': 'Beim Aufräumen werden zuerst die neuen Verknüpfungen erstellt, damit die alten bis zum Schluss funktionieren. Das Entfernen der alten Kopie braucht Administratorrechte; bist du hier kein Administrator, zeigt Kora den auszuführenden Befehl.',
  'servers.shortcuts.alsoPutShortcutDesktop': 'Auch eine Verknüpfung auf dem Desktop anlegen (Windows)',
  'servers.shortcuts.refreshShortcuts': 'Die Verknüpfungen aktualisieren',
  'servers.shortcuts.addShortcuts': 'Die Verknüpfungen hinzufügen',
} satisfies Record<keyof typeof en, string>

register('de', messages)
