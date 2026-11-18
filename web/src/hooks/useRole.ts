import { useQuery } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { Identity, Role } from '@/api/types'
import { t } from '@/i18n'
import '@/i18n/en/core/useRole'

const RANK: Record<Role, number> = { viewer: 0, operator: 1, admin: 2 }

/**
 * What the signed-in person may do, so the UI can hide or disable what would be refused. The server
 * checks every request anyway; this only avoids offering buttons that cannot work.
 *
 * - No identity in `/meta` (an older server, or the answer has not arrived yet) counts as a local admin.
 * - `can('operator')`: role is at least operator.
 * - `canDestroy`: admin, and (for another computer) the destructive actions are available: always for a
 *   Tailscale sign-in, and for a token sign-in once `kora panel remote-actions on` has been run. Installing a
 *   server needs only the admin role.
 */
export function useRole() {
  // Reads the `meta` answer the app shell already keeps fresh; it never fetches by itself.
  const meta = useQuery({ queryKey: ['meta'], queryFn: api.meta, enabled: false })
  const identity: Identity | null = meta.data?.identity ?? null
  const remote = meta.data?.remote ?? false
  const role: Role = identity?.role ?? 'admin'
  const destructiveAllowed = !remote || meta.data?.allow_remote_destructive === true
  return {
    identity,
    remote,
    role,
    can: (min: Role) => RANK[role] >= RANK[min],
    destructiveAllowed,
    canDestroy: RANK[role] >= RANK.admin && destructiveAllowed,
  }
}

/** The command that switches the destructive actions on for remote sign-ins. Change the name here only. */
export const ALLOW_REMOTE_COMMAND = 'kora panel remote-actions on'

/** Plain-words reason a button is off, for a tooltip or a note. */
export function whyNot(r: ReturnType<typeof useRole>, min: Role, destructive = false): string | null {
  if (!r.can(min)) return t('core.useRole.accountRoleCannotDo', { role: r.role, min })
  if (destructive && !r.destructiveAllowed) return t('core.useRole.turnedOffSigninsDo', { ALLOW_REMOTE_COMMAND })
  return null
}
