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
