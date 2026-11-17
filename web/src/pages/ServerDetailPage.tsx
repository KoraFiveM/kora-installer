import { Suspense, lazy, type ComponentType, type ReactNode } from 'react'
import { Link, useParams, useSearchParams } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { ArrowLeft } from 'lucide-react'
import { api, ApiError } from '@/api/client'
import type { ServerDetail } from '@/api/types'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Skeleton } from '@/components/Skeleton'
import { ErrorNotice } from '@/components/ErrorNotice'
import { ModeChip, StatusChip } from '@/components/Status'
import { useServerControl } from '@/hooks/useServerControl'
import NotFound from '@/pages/NotFound'
import { OverviewTab } from './server/OverviewTab'
import { ErrorBoundary } from '@/components/ErrorBoundary'
import { RouteLoading } from '@/components/RouteLoading'

// The Overview is the landing tab and loads with the page; every other tab is fetched when first opened.
const lazyTab = <K extends string>(load: () => Promise<Record<K, ComponentType<{ server: ServerDetail }>>>, name: K) => lazy(() => load().then((m) => ({ default: m[name] })))
const ConsoleTab = lazyTab(() => import('./server/ConsoleTab'), 'ConsoleTab')
const LogsTab = lazyTab(() => import('./server/LogsTab'), 'LogsTab')
const MonitoringTab = lazyTab(() => import('./server/MonitoringTab'), 'MonitoringTab')
const ResourcesTab = lazyTab(() => import('./server/ResourcesTab'), 'ResourcesTab')
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
  const dev = s.mode === 'development'
  // The Updates tab now lives in Resources; old links to ?tab=updates land there.
  const requested = params.get('tab') ?? 'overview'
  const current = requested === 'updates' ? 'resources' : requested
  return (
    <div className="space-y-5">
      <Link to="/" className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"><ArrowLeft className="size-4" /> All servers</Link>
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="font-heading text-2xl font-bold">{s.name}</h1>
        <ModeChip mode={s.mode} />
        <StatusChip status={s.status} pending={ctl.pending} />
      </div>
      <Tabs value={current} onValueChange={(t) => setParams({ tab: t }, { replace: true })}>
        <TabsList aria-label="Server sections">
          <TabsTrigger value="overview">Overview</TabsTrigger>
          <TabsTrigger value="console">Console</TabsTrigger>
          <TabsTrigger value="logs">Logs</TabsTrigger>
          <TabsTrigger value="monitoring">Monitoring</TabsTrigger>
          <TabsTrigger value="resources">Resources</TabsTrigger>
          <TabsTrigger value="assets">Paid assets</TabsTrigger>
          {dev && <TabsTrigger value="schedule">Schedule</TabsTrigger>}
          <TabsTrigger value="notifications">Notifications</TabsTrigger>
          <TabsTrigger value="ports">Ports</TabsTrigger>
          {dev && <TabsTrigger value="github">GitHub</TabsTrigger>}
          <TabsTrigger value="danger">Danger zone</TabsTrigger>
        </TabsList>
        <TabsContent value="overview"><OverviewTab server={s} ctl={ctl} /></TabsContent>
        <TabsContent value="console"><Lazy id="console"><ConsoleTab server={s} /></Lazy></TabsContent>
        <TabsContent value="logs"><Lazy id="logs"><LogsTab server={s} /></Lazy></TabsContent>
        <TabsContent value="monitoring"><Lazy id="monitoring"><MonitoringTab server={s} /></Lazy></TabsContent>
        <TabsContent value="resources"><Lazy id="resources"><ResourcesTab server={s} /></Lazy></TabsContent>
        <TabsContent value="assets"><Lazy id="assets"><PaidAssetsTab server={s} /></Lazy></TabsContent>
        {dev && <TabsContent value="schedule"><Lazy id="schedule"><ScheduleTab server={s} /></Lazy></TabsContent>}
        <TabsContent value="notifications"><Lazy id="notifications"><NotificationsTab server={s} /></Lazy></TabsContent>
        <TabsContent value="ports"><Lazy id="ports"><PortsTab server={s} /></Lazy></TabsContent>
        {dev && <TabsContent value="github"><Lazy id="github"><GithubTab server={s} /></Lazy></TabsContent>}
        <TabsContent value="danger"><Lazy id="danger"><DangerTab server={s} /></Lazy></TabsContent>
      </Tabs>
    </div>
  )
}
