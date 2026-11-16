import type { ReactElement } from 'react'
import { render } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router-dom'
import { AlertsProvider } from '@/lib/app'

export function renderApp(ui: ReactElement, route = '/') {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 }, mutations: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <AlertsProvider>
        <MemoryRouter initialEntries={[route]}>{ui}</MemoryRouter>
      </AlertsProvider>
    </QueryClientProvider>,
  )
}

export function jsonResponse(status: number, body?: unknown): Response {
  return new Response(body === undefined ? null : JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })
}

/** A controllable EventSource double. */
export class FakeEventSource {
  static instances: FakeEventSource[] = []
  listeners = new Map<string, ((e: MessageEvent) => void)[]>()
  onerror: ((e: Event) => void) | null = null
  closed = false
  constructor(public url: string) {
    FakeEventSource.instances.push(this)
  }
  addEventListener(t: string, fn: (e: MessageEvent) => void) {
    this.listeners.set(t, [...(this.listeners.get(t) ?? []), fn])
  }
  removeEventListener() {}
  close() {
    this.closed = true
  }
  emit(type: string, data: unknown) {
    const ev = new MessageEvent(type, { data: JSON.stringify(data) })
    this.listeners.get(type)?.forEach((f) => f(ev))
  }
  fail() {
    this.onerror?.(new Event('error'))
  }
}
