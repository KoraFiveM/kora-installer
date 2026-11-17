import { Suspense } from 'react'
import { NavLink, Outlet, useLocation } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { ListChecks, Network, Plus, Server, Settings, X } from 'lucide-react'
import { api } from '@/api/client'
import { useAlerts } from '@/lib/app'
import { useServerEvents } from '@/hooks/useServerEvents'
import { Alert } from '@/components/ui/alert'
import { AlertsBanner } from '@/components/AlertsBanner'
import { ErrorBoundary } from '@/components/ErrorBoundary'
import { RouteLoading } from '@/components/RouteLoading'
import { KoraLogo } from '@/components/KoraLogo'
import { useInstallBlock } from '@/pages/install/access'
import { cn } from '@/lib/utils'
import { useRouteAnnouncer } from '@/hooks/useRouteAnnouncer'
import { t, msg } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/core'

const nav = [
  { to: '/', label: msg('core.appShell.servers'), icon: Server, end: true },
  { to: '/fleet', label: msg('core.appShell.fleet'), icon: Network, end: false },
  { to: '/install', label: msg('core.appShell.installNewServer'), icon: Plus, end: false },
  { to: '/system', label: msg('core.appShell.systemCheck'), icon: ListChecks, end: false },
  { to: '/settings', label: msg('core.appShell.settings'), icon: Settings, end: false },
]

export function AppShell() {
  const loc = useLocation()
  const mainRef = useRouteAnnouncer(loc.pathname)
  const { alerts, dismiss } = useAlerts()
  const meta = useQuery({ queryKey: ['meta'], queryFn: api.meta, refetchInterval: 15000 })
  useServerEvents()
  const installBlock = useInstallBlock()
  const unreachable = meta.isError
  return (
    <div className="flex min-h-screen flex-col md:flex-row">
      <a href="#main" onClick={(e) => { e.preventDefault(); mainRef.current?.focus() }} className="sr-only focus:not-sr-only focus:fixed focus:left-2 focus:top-2 focus:z-[60] focus:rounded-md focus:bg-primary focus:px-4 focus:py-2 focus:text-sm focus:font-medium focus:text-primary-foreground">{t('core.appShell.skipMainContent')}</a>
      <aside className="border-b border-border bg-card md:min-h-screen md:w-60 md:shrink-0 md:border-b-0 md:border-r">
        <div className="px-5 py-4"><KoraLogo className="h-7 w-auto text-foreground" /></div>
        <nav aria-label={t('core.appShell.main')} className="flex flex-wrap gap-1 px-3 pb-3 md:flex-col md:flex-nowrap md:pb-0">
          {/* The fleet uses the tokens of other panels stored here, so it is only offered on this computer. */}
          {nav.filter((n) => !(n.to === '/fleet' && meta.data?.remote)).map(({ to, label, icon: Icon, end }) => {
            // Installing would be refused for this sign-in: say why here instead of after the whole wizard.
            if (to === '/install' && installBlock) {
              return (
                <span key={to} aria-disabled="true" title={installBlock.reason} className="flex cursor-not-allowed items-start gap-2 rounded-md px-3 py-2 text-sm font-medium text-muted-foreground/60">
                  <Icon className="mt-0.5 size-4 shrink-0" aria-hidden />
                  <span>{t(label)}<span className="block text-xs font-normal">{installBlock.short}</span></span>
                </span>
              )
            }
            return (
              <NavLink
                key={to}
                to={to}
                end={end}
                className={({ isActive }) =>
                  cn('flex items-center gap-2 rounded-md px-3 py-2 text-sm font-medium text-muted-foreground hover:bg-accent hover:text-foreground', isActive && 'bg-primary text-primary-foreground hover:bg-primary hover:text-primary-foreground')
                }
              >
                <Icon className="size-4" aria-hidden />
                {t(label)}
              </NavLink>
            )
          })}
        </nav>
      </aside>
      <div className="min-w-0 flex-1">
        <header className="flex items-center justify-between gap-4 border-b border-border px-6 py-3 text-sm">
          <span className="font-medium">{meta.data?.node ?? ' '}</span>
          <span className="text-muted-foreground">{meta.data ? `Kora ${meta.data.version}` : ''}</span>
        </header>
        <main id="main" ref={mainRef} tabIndex={-1} className="p-6 outline-none">
          <div className="mb-4 space-y-3 empty:hidden">
            {unreachable && <Alert tone="danger" title={t('core.appShell.koraAnswering')}>{rich('core.appShell.pageCannotReachKora', { code: (c) => <code>{c}</code> })}</Alert>}
            {alerts.map((a) => (
              <div key={a.id} className="relative">
                <Alert tone={a.tone}>{a.text}</Alert>
                <button className="absolute right-2 top-2 rounded-md p-1.5 text-muted-foreground hover:text-foreground" aria-label={t('core.appShell.dismissNotice')} onClick={() => dismiss(a.id)}>
                  <X className="size-4" aria-hidden />
                </button>
              </div>
            ))}
          </div>
          <div className="mb-4 empty:hidden"><AlertsBanner /></div>
          {meta.data?.auth_upgrade && location.pathname !== '/settings' && (
            <div className="mb-4">
              <Alert tone="warning" title={t('core.appShell.replaceTokenLink')}>{rich('core.appShell.signedTokenSecretLink', { navLink: (c) => <NavLink className="underline" to="/settings">{c}</NavLink> })}</Alert>
            </div>
          )}
          <ErrorBoundary resetKey={loc.pathname}>
            <Suspense fallback={<RouteLoading />}>
              <Outlet />
            </Suspense>
          </ErrorBoundary>
        </main>
      </div>
    </div>
  )
}
