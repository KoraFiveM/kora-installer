import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { ServerEvent, ServerSummary } from '@/api/types'

export const SERVER_POLL_MS = 5000

/**
 * Keep the server list live. Uses `GET /events` (SSE); if the stream is not available it polls the list instead.
 */
export function useServerEvents(): void {
  const qc = useQueryClient()
  useEffect(() => {
    let es: EventSource | null = null
    let timer: ReturnType<typeof setInterval> | null = null
    const startPolling = () => {
      if (timer) return
      timer = setInterval(() => {
        void qc.invalidateQueries({ queryKey: ['servers'] })
        void qc.invalidateQueries({ queryKey: ['server'] })
      }, SERVER_POLL_MS)
    }
    if (typeof EventSource === 'undefined') startPolling()
    else {
      es = new EventSource(api.eventsUrl())
      // The panel sends the whole list each time something changes.
      es.addEventListener('servers', (m) => {
        let list: ServerSummary[] | undefined
        try {
          list = (JSON.parse((m as MessageEvent<string>).data) as { servers?: ServerSummary[] }).servers
        } catch {
          return
        }
        if (!Array.isArray(list)) return
        const before = qc.getQueryData<ServerSummary[]>(['servers'])
        qc.setQueryData<ServerSummary[]>(['servers'], list)
        // Refresh the open server pages only when a status really changed (the first message repeats what is known).
        const was = new Map((before ?? []).map((s) => [s.name, s.status]))
        if (before && (before.length !== list.length || list.some((s) => was.get(s.name) !== s.status))) void qc.invalidateQueries({ queryKey: ['server'] })
      })
      // One server at a time (older panels).
      es.addEventListener('server', (m) => {
        let ev: ServerEvent
        try {
          ev = JSON.parse((m as MessageEvent<string>).data) as ServerEvent
        } catch {
          return
        }
        qc.setQueryData<ServerSummary[]>(['servers'], (old) => old?.map((s) => (s.name === ev.name ? { ...s, status: ev.status, badge: ev.badge } : s)))
        void qc.invalidateQueries({ queryKey: ['server', ev.name] })
      })
      es.onerror = () => {
        es?.close()
        es = null
        startPolling()
      }
    }
    return () => {
      es?.close()
      if (timer) clearInterval(timer)
    }
  }, [qc])
}
