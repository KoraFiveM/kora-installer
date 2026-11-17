import { useMemo, type ReactNode } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api, askForAdminNext } from '@/api/client'
import { useAlerts } from '@/lib/app'
import { ProblemActions, toProblem, type ActionContext, type ActionHandlers } from '@/lib/problemActions'

/**
 * The app-wide "Ask for administrator permission" / "Ask again" button (`request_admin`). Only on
 * Windows, where the panel can ask Windows itself (`GET /system` `admin_prompt`): it repeats what
 * failed, now telling the panel that the person asked, so Windows shows its permission prompt on
 * the computer that runs Kora (even for a remote session). Pages that know better (the install
 * wizard) register their own handler, which wins.
 */
export function AdminActions({ children }: { children: ReactNode }) {
  const sys = useQuery({ queryKey: ['system'], queryFn: api.system, staleTime: 60_000 })
  const qc = useQueryClient()
  const alerts = useAlerts()
  const handlers = useMemo<ActionHandlers>(() => {
    if (!sys.data?.admin_prompt) return {}
    return {
      request_admin: async (_a, ctx?: ActionContext) => {
        if (!ctx?.retry) return
        askForAdminNext()
        try {
          const r = await ctx.retry()
          const job = r && typeof r === 'object' && 'job' in r
          alerts.push('success', job ? 'Windows gave permission. Kora is doing it now.' : 'Windows gave permission and Kora made the change.')
        } catch (e) {
          alerts.push('warning', toProblem(e)?.message ?? 'Windows did not give permission.')
        } finally {
          void qc.invalidateQueries()
        }
      },
    }
  }, [sys.data?.admin_prompt, qc, alerts])
  return <ProblemActions handlers={handlers}>{children}</ProblemActions>
}
