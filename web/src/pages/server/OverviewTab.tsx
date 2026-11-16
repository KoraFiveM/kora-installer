import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { ExternalLink, FolderOpen } from 'lucide-react'
import { api } from '@/api/client'
import type { ServerDetail } from '@/api/types'
import type { useServerControl } from '@/hooks/useServerControl'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { CopyField } from '@/components/CopyButton'
import { ErrorNotice } from '@/components/ErrorNotice'
import { ServerControls } from '@/components/ServerControls'
import { UpdateChip } from '@/components/Status'

function Check({ ok, yes, no }: { ok: boolean; yes: string; no: string }) {
  return <Badge tone={ok ? 'success' : 'neutral'} dot>{ok ? yes : no}</Badge>
}

export function OverviewTab({ server: s, ctl }: { server: ServerDetail; ctl: ReturnType<typeof useServerControl> }) {
  const sys = useQuery({ queryKey: ['system'], queryFn: api.system })
  const [err, setErr] = useState<unknown>(null)
  const host = window.location.hostname
  const connect = s.connect ?? `${host}:${s.ports.game}`
  const tx = `http://${host}:${s.ports.txadmin}`
  return (
    <div className="grid gap-5 lg:grid-cols-2">
      <Card>
        <CardHeader><CardTitle>Status</CardTitle></CardHeader>
        <CardContent className="space-y-4">
          <dl className="grid grid-cols-[auto_1fr] items-center gap-x-6 gap-y-3 text-sm">
            <dt className="text-muted-foreground">Server</dt><dd><Check ok={s.running} yes="Running" no={s.status === 'missing' ? 'Folder missing' : 'Stopped'} /></dd>
            <dt className="text-muted-foreground">Game port {s.ports.game}</dt><dd><Check ok={s.game_listening} yes="Listening" no="Not listening" /></dd>
            <dt className="text-muted-foreground">txAdmin port {s.ports.txadmin}</dt><dd><Check ok={s.txadmin_up} yes="Answering" no="Not answering" /></dd>
            <dt className="text-muted-foreground">Updates</dt><dd><UpdateChip badge={s.badge} to={`/servers/${encodeURIComponent(s.name)}?tab=resources`} /></dd>
          </dl>
          <ServerControls name={s.name} status={s.status} busy={ctl.busy} run={ctl.run} size="default" />
          {s.status === 'missing' && <p className="text-sm text-amber">The server folder cannot be found. It may have been moved or deleted. You can remove this entry on the Danger zone tab.</p>}
        </CardContent>
      </Card>
      <Card>
        <CardHeader><CardTitle>Connect and manage</CardTitle></CardHeader>
        <CardContent className="space-y-4">
          <div className="space-y-1.5">
            <div className="text-sm text-muted-foreground">Connect address (players type this in the FiveM console, or use direct connect)</div>
            <CopyField value={connect} label="Connect address" />
          </div>
          <div className="space-y-1.5">
            <div className="text-sm text-muted-foreground">Server folder</div>
            <CopyField value={s.dir} label="Server folder" />
            {sys.data?.local && (
              <Button size="sm" onClick={async () => { setErr(null); try { await api.servers.openFolder(s.name) } catch (e) { setErr(e) } }}><FolderOpen /> Open folder</Button>
            )}
            {sys.data && !sys.data.local && <p className="text-xs text-muted-foreground">Opening the folder only works when you use this page on the computer that runs Kora.</p>}
          </div>
          <Button asChild variant="default" disabled={!s.txadmin_up}>
            <a href={tx} target="_blank" rel="noopener noreferrer"><ExternalLink /> Open txAdmin</a>
          </Button>
          {!s.txadmin_up && <p className="text-xs text-muted-foreground">txAdmin answers once the server is running.</p>}
          <ErrorNotice error={err} />
        </CardContent>
      </Card>
    </div>
  )
}
