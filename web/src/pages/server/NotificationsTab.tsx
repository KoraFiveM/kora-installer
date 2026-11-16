import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { ServerDetail } from '@/api/types'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'

export function NotificationsTab({ server: s }: { server: ServerDetail }) {
  const qc = useQueryClient()
  const q = useQuery({ queryKey: ['webhook', s.name], queryFn: () => api.webhook.get(s.name) })
  const [url, setUrl] = useState('')
  const refresh = () => qc.invalidateQueries({ queryKey: ['webhook', s.name] })
  const set = useMutation({ mutationFn: () => api.webhook.set(s.name, url.trim()), onSuccess: () => { setUrl(''); return refresh() } })
  const test = useMutation({ mutationFn: () => api.webhook.test(s.name) })
  const remove = useMutation({ mutationFn: () => api.webhook.remove(s.name), onSuccess: refresh })
  return (
    <Card className="max-w-2xl">
      <CardHeader><CardTitle>Discord notifications</CardTitle></CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm text-muted-foreground">Kora can post to a Discord channel when updates finish or something needs your attention. In Discord, open the channel settings, choose Integrations, then Webhooks, and copy the webhook address.</p>
        {q.data?.configured ? (
          <div className="space-y-3">
            <div className="text-sm">Current webhook: <code className="break-all rounded bg-ink px-2 py-1 font-mono text-xs">{q.data.redacted}</code></div>
            <div className="flex flex-wrap gap-2">
              <Button onClick={() => test.mutate()} disabled={test.isPending}>Send a test message</Button>
              <Button variant="destructive" onClick={() => remove.mutate()} disabled={remove.isPending}>Remove webhook</Button>
            </div>
          </div>
        ) : <p className="text-sm">No webhook is set.</p>}
        <form className="space-y-3" onSubmit={(e) => { e.preventDefault(); if (url.trim()) set.mutate() }}>
          <Field label={q.data?.configured ? 'Replace the webhook address' : 'Webhook address'} hint="The address is kept secret: it is never shown in full again.">
            <Input type="password" autoComplete="off" value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://discord.com/api/webhooks/..." />
          </Field>
          <Button type="submit" variant="default" disabled={!url.trim() || set.isPending}>{q.data?.configured ? 'Replace webhook' : 'Save webhook'}</Button>
        </form>
        {test.isSuccess && <Alert tone="success">Test message sent. Check your Discord channel.</Alert>}
        <ErrorNotice error={set.error ?? test.error ?? remove.error ?? q.error} />
      </CardContent>
    </Card>
  )
}
