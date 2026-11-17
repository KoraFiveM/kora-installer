import { useQuery } from '@tanstack/react-query'
import { api } from '@/api/client'

/**
 * Says whose GitHub account an action will use. It is the one `gh` on the computer that runs Kora is signed in as, shared by
 * everyone who uses this panel, so it is said wherever something is done on GitHub.
 */
export function ActingAs({ did }: { did?: string | null }) {
  const status = useQuery({ queryKey: ['github'], queryFn: api.github.status })
  const login = did ?? status.data?.user ?? null
  return (
    <p className="text-xs text-muted-foreground" data-testid="acting-as">
      {login ? <>This is done on GitHub as <strong className="text-foreground">{login}</strong>, the account this computer is signed in as. It is shared by everyone who uses this panel.</> : 'This is done as the GitHub account this computer is signed in as, shared by everyone who uses this panel.'}
    </p>
  )
}
