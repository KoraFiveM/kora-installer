import { useEffect, useRef, useState } from 'react'
import { api } from '@/api/client'
import type { Job, JobError } from '@/api/types'

export const JOB_POLL_MS = 1000
/** How many times in a row asking about a job may fail before it is given up on. One lost request must not turn a running job into a failed one. */
export const JOB_RETRIES = 4

/** The panel answered that the job does not exist (or may not be seen): asking again will not help. */
const gone = (e: unknown) => [403, 404].includes((e as { status?: number } | null)?.status ?? 0)

export type JobPhase = 'idle' | 'running' | 'done' | 'failed'

export interface JobView<R, E> {
  phase: JobPhase
  events: E[]
  result: R | null
  error: JobError | null
}

const IDLE: JobView<never, never> = { phase: 'idle', events: [], result: null, error: null }

// Finished jobs this page has already followed. A component that comes back on screen (the server
// page only mounts the open tab) shows the outcome at once: no flash of "running", no second request,
// and `onFinish` is not called a second time for the same job.
const FINISHED_MAX = 100
const finished = new Map<string, JobView<unknown, unknown>>()
function remember(id: string, view: JobView<unknown, unknown>) {
  finished.set(id, view)
  if (finished.size > FINISHED_MAX) finished.delete(finished.keys().next().value as string)
}
/** Forget the finished jobs (between tests). */
export function forgetJobs(): void {
  finished.clear()
}

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
 *
 * `onFinish` runs once per job, when this page first sees it finished. A job that ended while nothing was
 * following it (its tab was closed) is picked up, and reported, the next time its id is passed here.
 */
export function useJob<R = unknown, E = unknown>(jobId: string | null, onFinish?: (job: Job<R, E>) => void): JobView<R, E> {
  const [view, setView] = useState<JobView<R, E>>(() => ((jobId && finished.get(jobId)) || IDLE) as JobView<R, E>)
  const finish = useRef(onFinish)
  useEffect(() => {
    finish.current = onFinish
  })

  useEffect(() => {
    if (!jobId) {
      setView(IDLE as JobView<R, E>)
      return
    }
    const known = finished.get(jobId)
    if (known) {
      setView(known as JobView<R, E>)
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
      const end: JobView<R, E> = { phase: job.state === 'done' ? 'done' : 'failed', events: job.events.length ? job.events : seen, result: job.result, error: job.error }
      remember(jobId, end)
      setView(end)
      finish.current?.(job)
    }

    let misses = 0
    // The panel could not be asked. Try again a few times; only then say the job was lost. The job itself may
    // still be running, so `onFinish` is not called and nothing is remembered: a later look can still find it.
    const lost = (e: unknown, again: () => Promise<void>) => {
      if (closed) return
      if (!gone(e) && ++misses <= JOB_RETRIES) {
        timer = setTimeout(() => void again(), JOB_POLL_MS * misses)
        return
      }
      closed = true
      setView({ phase: 'failed', events: seen, result: null, error: { code: 'failed', message: gone(e) && e instanceof Error ? e.message : 'Kora lost contact with this job. It may still be running: reload the page to look again.' } })
    }

    const fetchFinal = async () => {
      if (closed) return
      try {
        const job = await api.jobs.get<R, E>(jobId)
        if (closed) return
        misses = 0
        if (job.state === 'running') {
          // The stream said "finished" a moment before the job record did; ask again shortly.
          timer = setTimeout(() => void fetchFinal(), JOB_POLL_MS)
        } else settle(job)
      } catch (e) {
        lost(e, fetchFinal)
      }
    }

    const poll = async () => {
      if (closed) return
      try {
        const job = await api.jobs.get<R, E>(jobId)
        if (closed) return
        misses = 0
        if (job.state === 'running') {
          setView({ phase: 'running', events: job.events, result: null, error: null })
          timer = setTimeout(() => void poll(), JOB_POLL_MS)
        } else settle(job)
      } catch (e) {
        lost(e, poll)
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
