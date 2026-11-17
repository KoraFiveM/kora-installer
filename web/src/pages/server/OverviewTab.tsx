import type { ServerDetail } from '@/api/types'
import type { useServerControl } from '@/hooks/useServerControl'
import { Badge } from '@/components/ui/badge'
import { OffNormalSummary } from '@/components/OffNormalSummary'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { CopyField } from '@/components/CopyButton'
import { CopyConnectButton, OpenFolderButton, TxAdminButton, useCanOpenFolder, useTxAdminLink } from '@/components/ServerLinks'
import { CONNECT_NOTE } from '@/lib/serverLinks'
import { LicenceKeyCard } from '@/components/ChangeLicenceKey'
import { MoreAddresses, ReachCard } from '@/components/ReachCard'
import { ServerControls } from '@/components/ServerControls'
import { UpdateChip } from '@/components/Status'
import { BackupChip, useBackupLines } from '@/components/BackupChip'
import { t } from '@/i18n'
import '@/i18n/en/servers'

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
        <CardHeader><CardTitle>{t('servers.overviewTab.status')}</CardTitle></CardHeader>
        <CardContent className="space-y-4">
          <dl className="grid grid-cols-[auto_1fr] items-center gap-x-6 gap-y-3 text-sm">
            <dt className="text-muted-foreground">{t('servers.overviewTab.server')}</dt><dd><Check ok={s.running} yes="Running" no={s.status === 'missing' ? t('servers.overviewTab.folderMissing') : 'Stopped'} /></dd>
            <dt className="text-muted-foreground">{t('servers.overviewTab.gamePortGame', { game: s.ports.game })}</dt><dd><Check ok={s.game_listening} yes="Listening" no={t('servers.overviewTab.notListening')} /></dd>
            <dt className="text-muted-foreground">{t('servers.overviewTab.txadminPortTxadmin', { txadmin: s.ports.txadmin })}</dt><dd><Check ok={s.txadmin_up} yes="Answering" no={t('servers.overviewTab.notAnswering')} /></dd>
            <dt className="text-muted-foreground">{t('servers.overviewTab.updates')}</dt><dd><UpdateChip badge={s.badge} to={`/servers/${encodeURIComponent(s.name)}?tab=resources`} /></dd>
            {backup && <><dt className="text-muted-foreground">{t('servers.overviewTab.backups')}</dt><dd><BackupChip server={s.name} summary={backup} /></dd></>}
          </dl>
          <ServerControls name={s.name} status={s.status} busy={ctl.busy} run={ctl.run} size="default" />
          <OffNormalSummary server={s} list />
          {s.status === 'missing' && <p className="text-sm text-amber">{t('servers.overviewTab.serverFolderCannotFound')}</p>}
        </CardContent>
      </Card>
      <Card>
        <CardHeader><CardTitle>{t('servers.overviewTab.connectManage')}</CardTitle></CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-wrap gap-2">
            <TxAdminButton name={s.name} port={s.ports.txadmin} up={s.txadmin_up} size="default" />
            <OpenFolderButton name={s.name} dir={s.dir} size="default" />
            <CopyConnectButton connect={s.connect} size="default" />
          </div>
          {!s.txadmin_up && <p className="text-xs text-muted-foreground">{t('servers.overviewTab.txadminAnswersOnceServer')}</p>}
          {s.txadmin_up && tx.hint && <p className="text-xs text-muted-foreground">{tx.hint}</p>}
          <div className="space-y-1.5">
            <div className="text-sm text-muted-foreground">{t('servers.overviewTab.connectAddressPlayersType')}</div>
            {s.connect ? <CopyField value={s.connect} label={t('servers.overviewTab.connectAddress')} /> : <p className="text-sm">{t('servers.overviewTab.koraCouldFindComputers', { game: s.ports.game })}</p>}
            <p className="text-xs text-muted-foreground">{CONNECT_NOTE}</p>
          </div>
          <MoreAddresses name={s.name} />
          <div className="space-y-1.5">
            <div className="text-sm text-muted-foreground">{t('servers.overviewTab.serverFolder')}</div>
            <CopyField value={s.dir} label={t('servers.overviewTab.serverFolder')} />
            {!canOpen && <p className="text-xs text-muted-foreground">{t('servers.overviewTab.openingFolderOnlyWorks')}</p>}
          </div>
        </CardContent>
      </Card>
      <div className="lg:col-span-2"><ReachCard name={s.name} onStart={s.status === 'stopped' ? () => void ctl.run('start') : undefined} /></div>
      <LicenceKeyCard server={s} busy={ctl.busy} restart={() => void ctl.run('restart')} />
    </div>
  )
}
