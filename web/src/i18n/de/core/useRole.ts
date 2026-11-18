import { register } from '../../index'
import type en from '../../en/core/useRole'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.useRole.accountRoleCannotDo': 'Dein Konto ({role}) kann das nicht. Bitte einen Administrator um {min}-Zugriff.',
  'core.useRole.turnedOffSigninsDo': 'Das ist für Anmeldungen ausgeschaltet, die nicht über Tailscale kommen, weil eine kopierbare Anmeldung (ein Token, eine gestohlene Browser-Sitzung) durchsickern könnte. Um es einzuschalten, führe das auf dem Computer aus, auf dem Kora läuft (über SSH geht auch): {ALLOW_REMOTE_COMMAND}. Oder melde dich als Administrator über Tailscale an oder nutze das Panel auf diesem Computer.',
} satisfies Record<keyof typeof en, string>

register('de', messages)
