import { useRole, whyNot } from '@/hooks/useRole'
import { t } from '@/i18n'
import '@/i18n/en/install/access'

/**
 * The command that lets token sign-ins delete servers (and use the other "destructive" actions).
 * It is run on the computer that runs Kora. This is the ONE place the command name lives.
 */
export const REMOTE_ACTIONS_COMMAND = 'kora panel remote-actions on'

export interface InstallBlock {
  kind: 'role'
  /** The full reason in plain words (tooltip text). */
  reason: string
  /** A few words to show next to a disabled button. */
  short: string
}

/**
 * Why the signed-in person cannot install a server (null = they can). Installing needs the admin role and
 * nothing else: remote administrators (Tailscale or token) install by default. Only deleting and the other
 * destructive actions wait for `remote actions` on a token sign-in.
 */
export function useInstallBlock(): InstallBlock | null {
  const role = useRole()
  const why = whyNot(role, 'admin')
  if (!why) return null
  return { kind: 'role', reason: why, short: t('install.access.needsAdministratorAccount') }
}
