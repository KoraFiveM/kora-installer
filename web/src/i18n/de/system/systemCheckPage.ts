import { register } from '../../index'
import type en from '../../en/system/systemCheckPage'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'system.systemCheckPage.systemCheck': 'Systemprüfung',
  'system.systemCheckPage.whatKoraNeedsComputer': 'Was Kora auf diesem Computer braucht, und ein Knopf, um alles zu beheben, was fehlt. Die meisten Korrekturen sind ein Knopf; wo eine einen Befehl braucht, wird er zum Kopieren angezeigt.',
  'system.systemCheckPage.nodeNodeCheckAgain': '{node, node}Erneut prüfen ',
  'system.systemCheckPage.loaderNodeCheckingComputer': '{loader2, node}Dieser Computer wird geprüft...',
  'system.systemCheckPage.everythingKoraNeedsHere': 'Alles, was Kora braucht, ist da.',
  'system.systemCheckPage.accountRoleLookBut': 'Dein Konto ({role}) kann sich das ansehen, aber nichts beheben. Bitte einen Administrator, diese Seite zu öffnen und die Knöpfe zu benutzen oder die angezeigten Befehle auszuführen.',
  'system.systemCheckPage.koraRunsCodeNode': 'Kora führt {code, node} aus',
  'system.systemCheckPage.fine': 'In Ordnung',
  'system.systemCheckPage.needsFixing': 'Muss behoben werden',
  'system.systemCheckPage.goodKnow': 'Gut zu wissen',
  'system.systemCheckPage.thingsNeedFixing': '{count, plural, one {# Sache muss behoben werden.} other {# Sachen müssen behoben werden.}}',
  'system.systemCheckPage.itemGit': 'Git',
  'system.systemCheckPage.itemGh': 'GitHub-CLI',
  'system.systemCheckPage.itemGithub': 'GitHub-Anmeldung',
  'system.systemCheckPage.itemDatabase': 'Datenbank',
  'system.systemCheckPage.itemAdmin': 'Administratorrechte',
  'system.systemCheckPage.itemService': 'Hintergrunddienst',
} satisfies Record<keyof typeof en, string>

register('de', messages)
