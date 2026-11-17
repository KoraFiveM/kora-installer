import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import { useRole } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Checkbox } from '@/components/ui/checkbox'
import { ErrorNotice } from '@/components/ErrorNotice'
import { t } from '@/i18n'
import '@/i18n/en/servers'

/** Status of the Kora shortcuts, and the one-click "Tidy up old shortcuts" (see `PUT /app`). */
function useShortcuts() {
  const qc = useQueryClient()
  const q = useQuery({ queryKey: ['app'], queryFn: api.app.status })
  const [desktop, setDesktop] = useState(true)
  const tidy = useMutation({
    mutationFn: () => api.app.install({ desktop_shortcut: desktop, dry_run: false }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['app'] }),
    // The per-user shortcuts are made even when the old copy still needs administrator rights.
    onError: () => qc.invalidateQueries({ queryKey: ['app'] }),
  })
  return { q, desktop, setDesktop, tidy }
}

/** A short notice on the start page, only while old shortcuts are around. */
export function TidyUpBanner() {
  const { q, tidy } = useShortcuts()
  const role = useRole()
  if (!q.data?.legacy || !role.can('admin') || tidy.isSuccess) return null
  return (
    <Alert tone="info">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <span>{t('servers.shortcuts.koraNowOpensFrom')}</span>
        <Button onClick={() => tidy.mutate()} disabled={tidy.isPending}>{t('servers.shortcuts.tidyUpOldShortcuts')}</Button>
      </div>
      <ErrorNotice error={tidy.error} />
    </Alert>
  )
}

export function ShortcutsCard() {
  const { q, desktop, setDesktop, tidy } = useShortcuts()
  const role = useRole()
  if (q.data && !q.data.supported) return null
  if (!role.can('admin')) return null
  const legacy = q.data?.legacy ?? false
  return (
    <Card>
      <CardHeader><CardTitle>{t('servers.shortcuts.koraShortcuts')}</CardTitle></CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">{t('servers.shortcuts.koraPanelBrowserShortcut')}</p>
        <div className="text-sm">{q.data?.shortcuts ? <Badge tone="success" dot>{t('servers.shortcuts.shortcutPlace')}</Badge> : <Badge tone="neutral" dot>{t('servers.shortcuts.noShortcutYet')}</Badge>}</div>
        {legacy && (
          <Alert tone="info">
            <p className="font-medium">{t('servers.shortcuts.olderKoraStillInstalled')}</p>
            <ul className="mt-1 list-disc pl-5 text-sm">{q.data?.legacy_items.map((i) => <li key={i}>{i}</li>)}</ul>
            <p className="mt-1 text-sm">{t('servers.shortcuts.tidyingUpMakesNew')}</p>
          </Alert>
        )}
        {tidy.isSuccess && <Alert tone="success">{tidy.data.message}</Alert>}
        <Checkbox label={t('servers.shortcuts.alsoPutShortcutDesktop')} checked={desktop} onChange={(e) => setDesktop(e.target.checked)} />
        <Button onClick={() => tidy.mutate()} disabled={tidy.isPending}>{legacy ? t('servers.shortcuts.tidyUpOldShortcuts') : q.data?.shortcuts ? t('servers.shortcuts.refreshShortcuts') : t('servers.shortcuts.addShortcuts')}</Button>
        <ErrorNotice error={tidy.error ?? q.error} />
      </CardContent>
    </Card>
  )
}
