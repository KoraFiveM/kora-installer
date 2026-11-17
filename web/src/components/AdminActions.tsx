import { useMemo, type ReactNode } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import { elevateInstallService } from '@/lib/shell'
import { useAlerts } from '@/lib/app'
import { ProblemActions, toProblem, type ActionContext, type ActionHandlers } from '@/lib/problemActions'

/** How long the installer may take before the panel stops waiting (the person answers Windows' prompt meanwhile). */
const INSTALL_WAIT_MS = 15 * 60_000

/**
 * The app-wide "Install Kora's administrator service" button (`install_admin_service`). Only on Windows, where
 * the panel can start the installer itself: it asks an administrator for approval ONCE, on the computer that
 * runs Kora (this is the one place a permission prompt remains), and then repeats what failed, which now
 * works without asking. On Linux ErrorNotice shows the command instead (`kora service install`).
 */
export function AdminActions({ children }: { children: ReactNode }) {
  const sys = useQuery({ queryKey: ['system'], queryFn: api.system, staleTime: 60_000 })
  const qc = useQueryClient()
  const alerts = useAlerts()
  const windows = sys.data?.platform === 'windows'
  const handlers = useMemo<ActionHandlers>(() => {
    if (!windows) return {}
    return {
      install_admin_service: async (_a, ctx?: ActionContext) => {
        try {
          // Inside the Kora desktop app the app itself launches `kora service install` with Windows' permission prompt in
          // front of its window (a background panel's prompt only flashes in the taskbar). The installer then runs on its
          // own, so wait until the service answers for this account. Outside the app (false): the panel starts it as a job.
          if (await elevateInstallService()) {
            alerts.push('info', 'The Kora installer is running. Windows asks for administrator permission in front of this window: choose Yes.')
            const until = Date.now() + INSTALL_WAIT_MS
            for (;;) {
              if ((await api.system()).admin_service?.state === 'ready') break
              if (Date.now() > until) throw new Error('The installer did not finish in time, or the permission was not given.')
              await new Promise((r) => setTimeout(r, 2000))
            }
          } else {
            const { job } = await api.adminService.install()
            alerts.push('info', 'The Kora installer is running. Windows asks for administrator permission on the computer that runs Kora: choose Yes.')
            const until = Date.now() + INSTALL_WAIT_MS
            for (;;) {
              const j = await api.jobs.get(job)
              if (j.state === 'done') break
              if (j.state === 'failed') throw new Error(j.error?.message ?? 'The installer stopped.')
              if (Date.now() > until) throw new Error('The installer did not finish in time. Check the computer that runs Kora.')
              await new Promise((r) => setTimeout(r, 1000))
            }
          }
          alerts.push('success', "Kora's administrator service is installed. Kora can now do this without asking.")
          void qc.invalidateQueries()
          if (ctx?.afterFix) await ctx.afterFix()
          else if (ctx?.retry) await ctx.retry()
        } catch (e) {
          alerts.push('warning', toProblem(e)?.message ?? "Kora's administrator service was not installed.")
        }
      },
    }
  }, [windows, qc, alerts])
  return <ProblemActions handlers={handlers}>{children}</ProblemActions>
}
