import { useEffect, useRef, useState } from 'react'
import { api } from '@/api/client'
import type { Job, JobError } from '@/api/types'

export const JOB_POLL_MS = 1000

export type JobPhase = 'idle' | 'running' | 'done' | 'failed'

export interface JobView<R, E> {
  phase: JobPhase
  events: E[]
  result: R | null
  error: JobError | null
}

const IDLE: JobView<never, never> = { phase: 'idle', events: [], result: null, error: null }

function parse<E>(data: string): E | null {
  try {
    return JSON.parse(data) as E
  } catch {
    return null
  }
}

/**
 * Follow a background job. Pass the job id (or null for "no job yet").
 * Uses `EventSource` on `/jobs/{id}/events` (events `progress`, `done`, `failed`; past events are replayed first)
 * and falls back to polling `GET /jobs/{id}` when the stream is unavailable. The final result and error always come
 * from `GET /jobs/{id}`, so they are the same whichever way the job was followed.
 */
export function useJob<R = unknown, E = unknown>(jobId: string | null, onFinish?: (job: Job<R, E>) => void): JobView<R, E> {
  const [view, setView] = useState<JobView<R, E>>(IDLE as JobView<R, E>)
  const finish = useRef(onFinish)
  useEffect(() => {
    finish.current = onFinish
  })

  useEffect(() => {
    if (!jobId) {
      setView(IDLE as JobView<R, E>)
      return
    }
    let closed = false
    let es: EventSource | null = null
    let timer: ReturnType<typeof setTimeout> | null = null
    const seen: E[] = []
    setView({ phase: 'running', events: [], result: null, error: null })

    const settle = (job: Job<R, E>) => {
      if (closed) return
      closed = true
      es?.close()
      if (timer) clearTimeout(timer)
      setView({ phase: job.state === 'done' ? 'done' : 'failed', events: job.events.length ? job.events : seen, result: job.result, error: job.error })
      finish.current?.(job)
    }

    const fetchFinal = async () => {
      try {
        const job = await api.jobs.get<R, E>(jobId)
        if (job.state === 'running') {
          // The stream said "finished" a moment before the job record did; ask again shortly.
          timer = setTimeout(() => void fetchFinal(), JOB_POLL_MS)
        } else settle(job)
      } catch (e) {
        if (closed) return
        closed = true
        setView({ phase: 'failed', events: seen, result: null, error: { message: e instanceof Error ? e.message : 'Lost track of this job.' } })
      }
    }

    const poll = async () => {
      if (closed) return
      try {
        const job = await api.jobs.get<R, E>(jobId)
        if (closed) return
        if (job.state === 'running') {
          setView({ phase: 'running', events: job.events, result: null, error: null })
          timer = setTimeout(() => void poll(), JOB_POLL_MS)
        } else settle(job)
      } catch (e) {
        if (closed) return
        closed = true
        setView({ phase: 'failed', events: seen, result: null, error: { message: e instanceof Error ? e.message : 'Lost track of this job.' } })
      }
    }

    if (typeof EventSource === 'undefined') {
      void poll()
    } else {
      es = new EventSource(api.jobs.eventsUrl(jobId))
      es.addEventListener('progress', (m) => {
        const ev = parse<E>((m as MessageEvent<string>).data)
        if (ev === null || closed) return
        seen.push(ev)
        setView({ phase: 'running', events: [...seen], result: null, error: null })
      })
      const end = () => {
        es?.close()
        void fetchFinal()
      }
      es.addEventListener('done', end)
      es.addEventListener('failed', end)
      es.onerror = () => {
        // The stream broke (or is not supported by a proxy): switch to polling.
        es?.close()
        es = null
        void poll()
      }
    }

    return () => {
      closed = true
      es?.close()
      if (timer) clearTimeout(timer)
    }
  }, [jobId])

  return view
}
