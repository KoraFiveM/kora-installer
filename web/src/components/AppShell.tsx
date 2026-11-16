import { NavLink, Outlet, useLocation } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { Network, Plus, Server, Settings, X } from 'lucide-react'
import { api } from '@/api/client'
import { useAlerts } from '@/lib/app'
import { useServerEvents } from '@/hooks/useServerEvents'
import { Alert } from '@/components/ui/alert'
import { AlertsBanner } from '@/components/AlertsBanner'
import { ErrorBoundary } from '@/components/ErrorBoundary'
import { cn } from '@/lib/utils'

const nav = [
  { to: '/', label: 'Servers', icon: Server, end: true },
  { to: '/fleet', label: 'Fleet', icon: Network, end: false },
  { to: '/install', label: 'Install new server', icon: Plus, end: false },
  { to: '/settings', label: 'Settings', icon: Settings, end: false },
]

export function AppShell() {
  const loc = useLocation()
  const { alerts, dismiss } = useAlerts()
  const meta = useQuery({ queryKey: ['meta'], queryFn: api.meta, refetchInterval: 15000 })
  useServerEvents()
  const unreachable = meta.isError
  return (
    <div className="flex min-h-screen flex-col md:flex-row">
      <aside className="border-b border-border bg-card md:min-h-screen md:w-60 md:shrink-0 md:border-b-0 md:border-r">
        <div className="px-5 py-4 font-heading text-xl font-bold tracking-tight">Kora</div>
        <nav aria-label="Main" className="flex gap-1 px-3 pb-3 md:flex-col md:pb-0">
          {nav.map(({ to, label, icon: Icon, end }) => (
            <NavLink
              key={to}
              to={to}
              end={end}
              className={({ isActive }) =>
                cn('flex items-center gap-2 rounded-md px-3 py-2 text-sm font-medium text-muted-foreground hover:bg-accent hover:text-foreground', isActive && 'bg-primary text-primary-foreground hover:bg-primary hover:text-primary-foreground')
              }
            >
              <Icon className="size-4" aria-hidden />
              {label}
            </NavLink>
          ))}
        </nav>
      </aside>
      <div className="min-w-0 flex-1">
        <header className="flex items-center justify-between gap-4 border-b border-border px-6 py-3 text-sm">
          <span className="font-medium">{meta.data?.node ?? ' '}</span>
          <span className="text-muted-foreground">{meta.data ? `Kora ${meta.data.version}` : ''}</span>
        </header>
        <div className="space-y-3 px-6 pt-4 empty:hidden" aria-label="Alerts">
          {unreachable && <Alert tone="danger" title="Kora is not answering">This page cannot reach Kora. Check that <code>kora panel</code> is still running. This page keeps trying.</Alert>}
          {alerts.map((a) => (
            <div key={a.id} className="relative">
              <Alert tone={a.tone}>{a.text}</Alert>
              <button className="absolute right-3 top-3 text-muted-foreground hover:text-foreground" aria-label="Dismiss" onClick={() => dismiss(a.id)}>
                <X className="size-4" />
              </button>
            </div>
          ))}
        </div>
        <div className="px-6 pt-4 empty:hidden"><AlertsBanner /></div>
        <main className="p-6">
          <ErrorBoundary resetKey={loc.pathname}>
            <Outlet />
          </ErrorBoundary>
        </main>
      </div>
    </div>
  )
}
