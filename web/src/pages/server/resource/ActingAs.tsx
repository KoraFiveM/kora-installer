import { useQuery } from '@tanstack/react-query'
import { api } from '@/api/client'
import { t } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/resources/actingAs'

/**
 * Says whose GitHub account an action will use. It is the one `gh` on the computer that runs Kora is signed in as, shared by
 * everyone who uses this panel, so it is said wherever something is done on GitHub.
 */
export function ActingAs({ did }: { did?: string | null }) {
  const status = useQuery({ queryKey: ['github'], queryFn: api.github.status })
  const login = did ?? status.data?.user ?? null
  return (
    <p className="text-xs text-muted-foreground" data-testid="acting-as">
      {login ? <>{rich('resources.actingAs.doneGithubStrongNode', { strong: <strong className="text-foreground">{login}</strong> })}</> : t('resources.actingAs.doneGithubAccountComputer')}
    </p>
  )
}
