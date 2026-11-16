import { useQuery } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { PanelAlert } from '@/api/types'

export const ALERT_POLL_MS = 10000

/** Monitoring alerts from the panel (not the on-screen notices of `useAlerts`). Polled; failures stay quiet. */
export function usePanelAlerts() {
  const q = useQuery({ queryKey: ['panel-alerts'], queryFn: api.alerts.list, refetchInterval: ALERT_POLL_MS, retry: false })
  const all = q.data ?? []
  /** Problems that are still there, or one-off news, and that nobody has acknowledged. */
  const open: PanelAlert[] = all.filter((a) => !a.acked && a.resolved === null)
  return { ...q, all, open }
}
