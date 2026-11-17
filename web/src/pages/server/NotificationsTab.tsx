import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { ServerDetail } from '@/api/types'
import { useRole, whyNot } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { t } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/servers/notificationsTab'

export function NotificationsTab({ server: s }: { server: ServerDetail }) {
  const qc = useQueryClient()
  const q = useQuery({ queryKey: ['webhook', s.name], queryFn: () => api.webhook.get(s.name) })
  const [url, setUrl] = useState('')
  const [confirm, setConfirm] = useState(false)
  const role = useRole()
  const noChange = whyNot(role, 'admin')
  const noTest = whyNot(role, 'operator')
  const refresh = () => qc.invalidateQueries({ queryKey: ['webhook', s.name] })
  const set = useMutation({ mutationFn: () => api.webhook.set(s.name, url.trim()), onSuccess: () => { setUrl(''); return refresh() } })
  const test = useMutation({ mutationFn: () => api.webhook.test(s.name) })
  const remove = useMutation({ mutationFn: () => api.webhook.remove(s.name), onSuccess: () => { setConfirm(false); return refresh() } })
  return (
    <Card className="max-w-2xl">
      <CardHeader><CardTitle>{t('servers.notificationsTab.discordNotifications')}</CardTitle></CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm text-muted-foreground">{t('servers.notificationsTab.koraPostDiscordChannel')}</p>
        {q.data?.configured ? (
          <div className="space-y-3">
            <div className="text-sm">{rich('servers.notificationsTab.currentWebhookCodeNode', { code: <code className="break-all rounded bg-ink px-2 py-1 font-mono text-xs">{q.data.redacted}</code> })}</div>
            <div className="flex flex-wrap gap-2">
              <Button onClick={() => test.mutate()} disabled={test.isPending || !!noTest} title={noTest ?? undefined}>{t('servers.notificationsTab.sendTestMessage')}</Button>
              <Button variant="destructive" onClick={() => setConfirm(true)} disabled={remove.isPending || !!noChange} title={noChange ?? undefined}>{t('servers.notificationsTab.removeWebhook')}</Button>
            </div>
          </div>
        ) : <p className="text-sm">{t('servers.notificationsTab.noWebhookSet')}</p>}
        <form className="space-y-3" onSubmit={(e) => { e.preventDefault(); if (url.trim()) set.mutate() }}>
          <Field label={q.data?.configured ? t('servers.notificationsTab.replaceWebhookAddress') : t('servers.notificationsTab.webhookAddress')} hint={t('servers.notificationsTab.addressKeptSecretNever')}>
            <Input type="password" autoComplete="off" disabled={!!noChange} value={url} onChange={(e) => setUrl(e.target.value)} placeholder={t('servers.notificationsTab.httpsDiscordComApi')} />
          </Field>
          <Button type="submit" variant="default" disabled={!url.trim() || set.isPending || !!noChange} title={noChange ?? undefined}>{q.data?.configured ? t('servers.notificationsTab.replaceWebhook') : t('servers.notificationsTab.saveWebhook')}</Button>
        </form>
        {noChange && <p className="text-xs text-muted-foreground">{noChange}</p>}
        {test.isSuccess && <Alert tone="success">{t('servers.notificationsTab.testMessageSentCheck')}</Alert>}
        <ErrorNotice error={set.error ?? test.error ?? remove.error ?? q.error} />
        <ConfirmDialog
          open={confirm}
          onOpenChange={setConfirm}
          title={t('servers.notificationsTab.removeWebhook2')}
          description={t('servers.notificationsTab.alertsStopBeingSent')}
          confirmLabel={t('servers.notificationsTab.removeWebhook')}
          destructive
          busy={remove.isPending}
          onConfirm={() => remove.mutate()}
        />
      </CardContent>
    </Card>
  )
}
