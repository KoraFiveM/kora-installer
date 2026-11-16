import { useRole, whyNot } from '@/hooks/useRole'

/**
 * The command that lets remote sign-ins install servers (and use the other "destructive" actions).
 * It is run on the computer that runs Kora. This is the ONE place the command name lives.
 */
export const REMOTE_ACTIONS_COMMAND = 'kora panel remote-actions on'

export interface InstallBlock {
  /** `remote`: an admin, but installing is switched off for remote sign-ins. `role`: not an admin. */
  kind: 'remote' | 'role'
  /** The full reason in plain words (tooltip text). */
  reason: string
  /** A few words to show next to a disabled button. */
  short: string
}

/** Why the signed-in person cannot install a server (null = they can). Installing is admin-only and off for remote sign-ins unless switched on. */
export function useInstallBlock(): InstallBlock | null {
  const role = useRole()
  const why = whyNot(role, 'admin', true)
  if (!why) return null
  if (role.can('admin') && !role.destructiveAllowed) {
    return {
      kind: 'remote',
      reason: `Installing a server is turned off for remote sign-ins. To turn it on, run this on the computer that runs Kora (over SSH is fine): ${REMOTE_ACTIONS_COMMAND}. Then reload this page.`,
      short: 'Turned off for remote sign-ins.',
    }
  }
  return { kind: 'role', reason: why, short: 'Needs an administrator account.' }
}
