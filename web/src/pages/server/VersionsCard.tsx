import { useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { ServerDetail, Versions } from '@/api/types'
import { useRole, whyNot } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Radio } from '@/components/ui/checkbox'
import { ErrorNotice } from '@/components/ErrorNotice'
import { t } from '@/i18n'
import '@/i18n/en/servers/versionsCard'

/**
 * "Which versions": released versions (pinned, stable) or the latest changes (the main branch). A setting of its own,
 * not tied to whether the resources are plain copies or your own. Changing it moves nothing: the next check for updates
 * shows what would change, and nothing happens until the update is applied.
 */
export function VersionsCard({ server: s }: { server: ServerDetail }) {
  const qc = useQueryClient()
  const blocked = whyNot(useRole(), 'admin')
  const [pick, setPick] = useState<Versions>(s.versions)
  const save = useMutation({ mutationFn: () => api.copies.setVersions(s.name, pick), onSuccess: () => { void qc.invalidateQueries({ queryKey: ['server', s.name] }); void qc.invalidateQueries({ queryKey: ['servers'] }) } })
  return (
    <Card>
      <CardHeader><CardTitle>{t('servers.versionsCard.whichVersions')}</CardTitle></CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">{t('servers.versionsCard.resourcesHereValueSets', { style: s.source })}</p>
        <fieldset className="space-y-2">
          <legend className="sr-only">{t('servers.versionsCard.whichVersionsServerFollows')}</legend>
          <Radio name="versions-setting" checked={pick === 'released'} onChange={() => setPick('released')} label={t('servers.versionsCard.releasedVersionsRecommendedSta')} hint={t('servers.versionsCard.eachResourcePinnedTested')} />
          <Radio name="versions-setting" checked={pick === 'latest'} onChange={() => setPick('latest')} label={t('servers.versionsCard.latestChangesMainBranch')} hint={t('servers.versionsCard.eachResourceFollowsNewest')} />
        </fieldset>
        <Button disabled={pick === s.versions || save.isPending || !!blocked} title={blocked ?? undefined} onClick={() => save.mutate()}>{t('servers.versionsCard.save')}</Button>
        {save.data && <Alert tone="info">{save.data.message}</Alert>}
        <ErrorNotice error={save.error} />
      </CardContent>
    </Card>
  )
}
