import { Link, useParams, useSearchParams } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { ArrowLeft } from 'lucide-react'
import { api, ApiError } from '@/api/client'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Skeleton } from '@/components/Skeleton'
import { ErrorNotice } from '@/components/ErrorNotice'
import { ModeChip, StatusChip } from '@/components/Status'
import { useServerControl } from '@/hooks/useServerControl'
import NotFound from '@/pages/NotFound'
import { OverviewTab } from './server/OverviewTab'
import { ConsoleTab } from './server/ConsoleTab'
import { LogsTab } from './server/LogsTab'
import { UpdatesTab } from './server/UpdatesTab'
import { ResourcesTab } from './server/ResourcesTab'
import { ScheduleTab } from './server/ScheduleTab'
import { NotificationsTab } from './server/NotificationsTab'
import { PortsTab } from './server/PortsTab'
import { MonitoringTab } from './server/MonitoringTab'
import { GithubTab } from './server/GithubTab'
import { DangerTab } from './server/DangerTab'

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
  const tab = params.get('tab') ?? 'overview'
  return (
    <div className="space-y-5">
      <Link to="/" className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"><ArrowLeft className="size-4" /> All servers</Link>
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="font-heading text-2xl font-bold">{s.name}</h1>
        <ModeChip mode={s.mode} />
        <StatusChip status={s.status} pending={ctl.pending} />
      </div>
      <Tabs value={tab} onValueChange={(t) => setParams({ tab: t }, { replace: true })}>
        <TabsList aria-label="Server sections">
          <TabsTrigger value="overview">Overview</TabsTrigger>
          <TabsTrigger value="console">Console</TabsTrigger>
          <TabsTrigger value="logs">Logs</TabsTrigger>
          <TabsTrigger value="updates">Updates</TabsTrigger>
          <TabsTrigger value="monitoring">Monitoring</TabsTrigger>
          <TabsTrigger value="resources">Resources</TabsTrigger>
          {dev && <TabsTrigger value="schedule">Schedule</TabsTrigger>}
          <TabsTrigger value="notifications">Notifications</TabsTrigger>
          <TabsTrigger value="ports">Ports</TabsTrigger>
          {dev && <TabsTrigger value="github">GitHub</TabsTrigger>}
          <TabsTrigger value="danger">Danger zone</TabsTrigger>
        </TabsList>
        <TabsContent value="overview"><OverviewTab server={s} ctl={ctl} /></TabsContent>
        <TabsContent value="console"><ConsoleTab server={s} /></TabsContent>
        <TabsContent value="logs"><LogsTab server={s} /></TabsContent>
        <TabsContent value="updates"><UpdatesTab server={s} /></TabsContent>
        <TabsContent value="monitoring"><MonitoringTab server={s} /></TabsContent>
        <TabsContent value="resources"><ResourcesTab server={s} /></TabsContent>
        {dev && <TabsContent value="schedule"><ScheduleTab server={s} /></TabsContent>}
        <TabsContent value="notifications"><NotificationsTab server={s} /></TabsContent>
        <TabsContent value="ports"><PortsTab server={s} /></TabsContent>
        {dev && <TabsContent value="github"><GithubTab server={s} /></TabsContent>}
        <TabsContent value="danger"><DangerTab server={s} /></TabsContent>
      </Tabs>
    </div>
  )
}
