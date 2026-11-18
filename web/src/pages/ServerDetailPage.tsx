import { Suspense, lazy, type ComponentType, type ReactNode } from 'react'
import { Link, useParams, useSearchParams } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { ArrowLeft } from 'lucide-react'
import { api, ApiError } from '@/api/client'
import type { ServerDetail } from '@/api/types'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Skeleton } from '@/components/Skeleton'
import { ErrorNotice } from '@/components/ErrorNotice'
import { RunChip, StatusChip, StyleChip } from '@/components/Status'
import { useServerControl } from '@/hooks/useServerControl'
import NotFound from '@/pages/NotFound'
import { OverviewTab } from './server/OverviewTab'
import { ErrorBoundary } from '@/components/ErrorBoundary'
import { RouteLoading } from '@/components/RouteLoading'
import { t as tt } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/servers/serverDetailPage'

// The Overview is the landing tab and loads with the page; every other tab is fetched when first opened.
const lazyTab = <K extends string>(load: () => Promise<Record<K, ComponentType<{ server: ServerDetail }>>>, name: K) => lazy(() => load().then((m) => ({ default: m[name] })))
const ConsoleTab = lazyTab(() => import('./server/ConsoleTab'), 'ConsoleTab')
const LogsTab = lazyTab(() => import('./server/LogsTab'), 'LogsTab')
const MonitoringTab = lazyTab(() => import('./server/MonitoringTab'), 'MonitoringTab')
const ResourcesTab = lazyTab(() => import('./server/ResourcesTab'), 'ResourcesTab')
const BackupsTab = lazyTab(() => import('./server/BackupsTab'), 'BackupsTab')
const PaidAssetsTab = lazyTab(() => import('./server/PaidAssetsTab'), 'PaidAssetsTab')
const ScheduleTab = lazyTab(() => import('./server/ScheduleTab'), 'ScheduleTab')
const NotificationsTab = lazyTab(() => import('./server/NotificationsTab'), 'NotificationsTab')
const PortsTab = lazyTab(() => import('./server/PortsTab'), 'PortsTab')
const GithubTab = lazyTab(() => import('./server/GithubTab'), 'GithubTab')
const DangerTab = lazyTab(() => import('./server/DangerTab'), 'DangerTab')

/** One tab's content: its own loading state, and its own error screen so a tab that cannot load leaves the rest usable. */
function Lazy({ id, children }: { id: string; children: ReactNode }) {
  return (
    <ErrorBoundary resetKey={id}>
      <Suspense fallback={<RouteLoading />}>{children}</Suspense>
    </ErrorBoundary>
  )
}

export default function ServerDetailPage() {
  const { name = '' } = useParams()
  const [params, setParams] = useSearchParams()
  const q = useQuery({ queryKey: ['server', name], queryFn: () => api.servers.get(name) })
  const ctl = useServerControl(name)
  if (q.isLoading) return <Skeleton className="h-64" />
  if (q.error instanceof ApiError && q.error.status === 404) return <NotFound />
  if (q.isError || !q.data) return <ErrorNotice error={q.error} />
  const s = q.data
  // The Updates tab now lives in Resources; old links to ?tab=updates land there.
  const requested = params.get('tab') ?? 'overview'
  const current = requested === 'updates' ? 'resources' : requested
  return (
    <div className="space-y-5">
      <Link to="/" className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground">{rich('servers.serverDetailPage.arrowleftNodeAllServers', { arrowLeft: <ArrowLeft className="size-4" /> })}</Link>
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="font-heading text-2xl font-bold">{s.name}</h1>
        <StyleChip source={s.source} versions={s.versions} />
        <RunChip mode={s.run_mode} label={s.run_label} />
        <StatusChip status={s.status} pending={ctl.pending} />
      </div>
      <Tabs value={current} onValueChange={(t) => setParams({ tab: t }, { replace: true })}>
        <TabsList aria-label={tt('servers.serverDetailPage.serverSections')}>
          <TabsTrigger value="overview">{tt('servers.serverDetailPage.overview')}</TabsTrigger>
          <TabsTrigger value="console">{tt('servers.serverDetailPage.console')}</TabsTrigger>
          <TabsTrigger value="logs">{tt('servers.serverDetailPage.logs')}</TabsTrigger>
          <TabsTrigger value="monitoring">{tt('servers.serverDetailPage.monitoring')}</TabsTrigger>
          <TabsTrigger value="resources">{tt('servers.serverDetailPage.resources')}</TabsTrigger>
          <TabsTrigger value="assets">{tt('servers.serverDetailPage.paidAssets')}</TabsTrigger>
          <TabsTrigger value="backups">{tt('servers.serverDetailPage.backups')}</TabsTrigger>
          <TabsTrigger value="schedule">{tt('servers.serverDetailPage.schedule')}</TabsTrigger>
          <TabsTrigger value="notifications">{tt('servers.serverDetailPage.notifications')}</TabsTrigger>
          <TabsTrigger value="ports">{tt('servers.serverDetailPage.ports')}</TabsTrigger>
          <TabsTrigger value="github">GitHub</TabsTrigger>
          <TabsTrigger value="danger">{tt('servers.serverDetailPage.dangerZone')}</TabsTrigger>
        </TabsList>
        <TabsContent value="overview"><OverviewTab server={s} ctl={ctl} /></TabsContent>
        <TabsContent value="console"><Lazy id="console"><ConsoleTab server={s} /></Lazy></TabsContent>
        <TabsContent value="logs"><Lazy id="logs"><LogsTab server={s} /></Lazy></TabsContent>
        <TabsContent value="monitoring"><Lazy id="monitoring"><MonitoringTab server={s} /></Lazy></TabsContent>
        <TabsContent value="resources"><Lazy id="resources"><ResourcesTab server={s} /></Lazy></TabsContent>
        <TabsContent value="assets"><Lazy id="assets"><PaidAssetsTab server={s} /></Lazy></TabsContent>
        <TabsContent value="backups"><Lazy id="backups"><BackupsTab server={s} /></Lazy></TabsContent>
        <TabsContent value="schedule"><Lazy id="schedule"><ScheduleTab server={s} /></Lazy></TabsContent>
        <TabsContent value="notifications"><Lazy id="notifications"><NotificationsTab server={s} /></Lazy></TabsContent>
        <TabsContent value="ports"><Lazy id="ports"><PortsTab server={s} /></Lazy></TabsContent>
        <TabsContent value="github"><Lazy id="github"><GithubTab server={s} /></Lazy></TabsContent>
        <TabsContent value="danger"><Lazy id="danger"><DangerTab server={s} /></Lazy></TabsContent>
      </Tabs>
    </div>
  )
}
