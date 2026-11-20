import { useQuery } from '@tanstack/react-query'
import { api } from '@/api/client'
import { useRole } from '@/hooks/useRole'

/**
 * Is Kora set up well enough to do anything: a Cfx.re licence key is saved, and GitHub is signed in?
 * `blocked` is only true once both answers have arrived and one of them says "no" (a slow or failed
 * answer never locks the panel), and only for an administrator, who is the one who can fix it.
 */
export function useSetup() {
  const role = useRole()
  const github = useQuery({ queryKey: ['github'], queryFn: api.github.status, retry: false })
  const keys = useQuery({ queryKey: ['keys'], queryFn: api.keys.list, retry: false })
  const githubReady = github.data ? github.data.signed_in && github.data.state === 'ready' : null
  const cfxReady = keys.data ? keys.data.keys.length > 0 : null
  const blocked = role.can('admin') && (githubReady === false || cfxReady === false)
  return { cfxReady, githubReady, blocked }
}

/** Pages that stay open while setup is unfinished: the ones where setup is done, and the help. */
export const SETUP_OPEN = ['/settings', '/system', '/help']
