import type { ServerDetail } from '@/api/types'
import type { useServerControl } from '@/hooks/useServerControl'
import { Badge } from '@/components/ui/badge'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { CopyField } from '@/components/CopyButton'
import { CopyConnectButton, OpenFolderButton, TxAdminButton, useCanOpenFolder, useTxAdminLink } from '@/components/ServerLinks'
import { CONNECT_NOTE } from '@/lib/serverLinks'
import { LicenceKeyCard } from '@/components/ChangeLicenceKey'
import { MoreAddresses, ReachCard } from '@/components/ReachCard'
import { ServerControls } from '@/components/ServerControls'
import { UpdateChip } from '@/components/Status'
import { BackupChip, useBackupLines } from '@/components/BackupChip'

function Check({ ok, yes, no }: { ok: boolean; yes: string; no: string }) {
  return <Badge tone={ok ? 'success' : 'neutral'} dot>{ok ? yes : no}</Badge>
}

export function OverviewTab({ server: s, ctl }: { server: ServerDetail; ctl: ReturnType<typeof useServerControl> }) {
  const canOpen = useCanOpenFolder()
  const tx = useTxAdminLink(s.ports.txadmin)
  const backup = useBackupLines().by.get(s.name)
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
            {backup && <><dt className="text-muted-foreground">Backups</dt><dd><BackupChip server={s.name} summary={backup} /></dd></>}
          </dl>
          <ServerControls name={s.name} status={s.status} busy={ctl.busy} run={ctl.run} size="default" />
          {s.status === 'missing' && <p className="text-sm text-amber">The server folder cannot be found. It may have been moved or deleted. You can remove this entry on the Danger zone tab.</p>}
        </CardContent>
      </Card>
      <Card>
        <CardHeader><CardTitle>Connect and manage</CardTitle></CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-wrap gap-2">
            <TxAdminButton name={s.name} port={s.ports.txadmin} up={s.txadmin_up} size="default" />
            <OpenFolderButton name={s.name} dir={s.dir} size="default" />
            <CopyConnectButton connect={s.connect} size="default" />
          </div>
          {!s.txadmin_up && <p className="text-xs text-muted-foreground">txAdmin answers once the server is running.</p>}
          {s.txadmin_up && tx.hint && <p className="text-xs text-muted-foreground">{tx.hint}</p>}
          <div className="space-y-1.5">
            <div className="text-sm text-muted-foreground">Connect address (players type this in the FiveM console, or use direct connect)</div>
            {s.connect ? <CopyField value={s.connect} label="Connect address" /> : <p className="text-sm">Kora could not find this computer's network address. Players connect to this computer's address with port {s.ports.game}.</p>}
            <p className="text-xs text-muted-foreground">{CONNECT_NOTE}</p>
          </div>
          <MoreAddresses name={s.name} />
          <div className="space-y-1.5">
            <div className="text-sm text-muted-foreground">Server folder</div>
            <CopyField value={s.dir} label="Server folder" />
            {!canOpen && <p className="text-xs text-muted-foreground">Opening the folder only works on the computer that runs Kora, at its screen. From here, copy the path.</p>}
          </div>
        </CardContent>
      </Card>
      <div className="lg:col-span-2"><ReachCard name={s.name} onStart={s.status === 'stopped' ? () => void ctl.run('start') : undefined} /></div>
      <LicenceKeyCard server={s} busy={ctl.busy} restart={() => void ctl.run('restart')} />
    </div>
  )
}
