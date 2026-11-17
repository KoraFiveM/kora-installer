import { useQuery } from '@tanstack/react-query'
import { CheckCircle2, Info, Loader2, RefreshCw, XCircle } from 'lucide-react'
import { api } from '@/api/client'
import type { CheckItem, CheckState, ToolStatus } from '@/api/types'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { ErrorNotice } from '@/components/ErrorNotice'

const BADGE: Record<CheckState, { tone: 'success' | 'amber' | 'neutral'; text: string }> = {
  ok: { tone: 'success', text: 'Fine' },
  problem: { tone: 'amber', text: 'Needs fixing' },
  info: { tone: 'neutral', text: 'Good to know' },
}

function StateIcon({ state }: { state: CheckState }) {
  if (state === 'ok') return <CheckCircle2 className="size-5 shrink-0 text-success" aria-hidden />
  if (state === 'problem') return <XCircle className="size-5 shrink-0 text-amber" aria-hidden />
  return <Info className="size-5 shrink-0 text-steel" aria-hidden />
}

/** `Where Kora found it` for the Git and GitHub CLI lines, from `GET /tools`. */
function whereIs(item: CheckItem, tools: ToolStatus[] | undefined): string | null {
  const t = tools?.find((x) => x.name === item.id)
  if (!t?.installed || !t.path) return null
  return t.path
}

/**
 * The System check: what Kora needs on this computer (Git, the GitHub CLI, the GitHub sign-in, the database,
 * administrator rights), each in plain words with a button that fixes it where something is wrong. The buttons are
 * the app-wide problem actions (components/ToolchainActions.tsx), so they work the same here as in an error.
 */
export default function SystemCheckPage() {
  const check = useQuery({ queryKey: ['system-check'], queryFn: api.systemCheck.get, staleTime: 0 })
  const tools = useQuery({ queryKey: ['tools'], queryFn: api.tools.status, staleTime: 0 })
  const items = check.data?.items ?? []
  const problems = items.filter((i) => i.state === 'problem').length
  return (
    <div className="mx-auto max-w-3xl space-y-6">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="font-heading text-2xl font-semibold">System check</h1>
          <p className="mt-1 text-sm text-muted-foreground">What Kora needs on this computer, and a button to fix anything that is missing. Nothing here needs a terminal.</p>
        </div>
        <Button variant="secondary" onClick={() => void Promise.all([check.refetch(), tools.refetch()])} disabled={check.isFetching}>
          {check.isFetching ? <Loader2 className="animate-spin" aria-hidden /> : <RefreshCw aria-hidden />}Check again
        </Button>
      </header>

      {check.isLoading && <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground"><Loader2 className="size-4 animate-spin" aria-hidden />Checking this computer...</p>}
      {check.isError && <ErrorNotice error={check.error} onRetry={() => void check.refetch()} />}

      {check.data && (
        <p role="status" className="text-sm">
          {problems === 0 ? <span className="text-success">Everything Kora needs is here.</span> : <span className="text-amber">{problems === 1 ? '1 thing needs fixing.' : `${problems} things need fixing.`}</span>}
        </p>
      )}

      <ul className="space-y-3">
        {items.map((i) => {
          const where = whereIs(i, tools.data)
          return (
            <li key={i.id}>
              <Card>
                <CardContent className="space-y-3 pt-5">
                  <div className="flex items-start gap-3">
                    <StateIcon state={i.state} />
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <h2 className="font-semibold">{i.label}</h2>
                        <Badge tone={BADGE[i.state].tone} dot>{BADGE[i.state].text}</Badge>
                      </div>
                      <p className="mt-1 text-sm text-muted-foreground">{i.summary}</p>
                      {where && <p className="mt-1 break-all text-xs text-muted-foreground">Kora runs <code className="font-mono">{where}</code></p>}
                    </div>
                  </div>
                  {i.problem && <ErrorNotice error={i.problem} />}
                </CardContent>
              </Card>
            </li>
          )
        })}
      </ul>
    </div>
  )
}
