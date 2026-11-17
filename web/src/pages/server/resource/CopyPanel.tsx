import { useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { Loader2 } from 'lucide-react'
import { api } from '@/api/client'
import type { ForkOutcome, ResourcePage } from '@/api/types'
import { useJob } from '@/hooks/useJob'
import { useRole, whyNot } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { ErrorNotice } from '@/components/ErrorNotice'
import { ActingAs } from './ActingAs'
import { t } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/resources'

/**
 * "Make my own copy (fork)" for a resource that is a plain clone, and for the whole server. Features that need your own
 * copy (push, open a pull request, keeping up with the original) wait for it.
 */
export function CopyPanel({ page }: { page: ResourcePage }) {
  const qc = useQueryClient()
  const s = page.server.name
  const noAdmin = whyNot(useRole(), 'admin')
  const [job, setJob] = useState<string | null>(null)
  const [err, setErr] = useState<unknown>(null)
  const run = useJob<ForkOutcome>(job, () => { void qc.invalidateQueries({ queryKey: ['server', s] }); void qc.invalidateQueries({ queryKey: ['servers'] }) })
  const start = async (resources?: string[]) => {
    setErr(null)
    try { setJob((await api.copies.fork(s, { resources })).job) } catch (e) { setErr(e) }
  }
  if (page.repos?.fork) {
    return (
      <p className="text-sm text-muted-foreground">{rich('resources.copyPanel.ownCopyNodeNode', { a: <a className="text-info underline" href={`https://github.com/${page.repos.fork}`} target="_blank" rel="noopener noreferrer">{page.repos.fork}</a>, node: page.repos.original ? <>{t('resources.copyPanel.copyOriginal', { original: page.repos.original })}</> : null })}</p>
    )
  }
  if (!page.repos) return null
  return (
    <Card id="own-copy">
      <CardHeader><CardTitle>{t('resources.copyPanel.ownCopy')}</CardTitle></CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">{rich('resources.copyPanel.resourcePlainCopyStrong', { strong: <strong>{page.repos.original ?? t('resources.copyPanel.theOriginal')}</strong> })}</p>
        <div className="flex flex-wrap gap-2">
          <Button variant="default" disabled={run.phase === 'running' || !!noAdmin} title={noAdmin ?? undefined} onClick={() => void start([page.id])}>{rich('resources.copyPanel.nodeNodeMakeMy', { node: run.phase === 'running' && <Loader2 className="animate-spin" aria-hidden /> })}</Button>
          <Button disabled={run.phase === 'running' || !!noAdmin} title={noAdmin ?? undefined} onClick={() => void start()}>{t('resources.copyPanel.doWholeServer')}</Button>
        </div>
        <ActingAs />
        <ErrorNotice error={err ?? (run.phase === 'failed' ? run.error : null)} />
        {run.phase === 'done' && run.result && (
          <Alert tone={run.result.failed.length ? 'warning' : 'success'} title={run.result.failed.length ? t('resources.copyPanel.someCouldCopied') : t('resources.copyPanel.done')}>
            {run.result.forked.map((f) => <p key={f.resource}>{f.message}</p>)}
            {run.result.failed.map((f) => <p key={f.resource}>{f.resource}: {f.problem.message}</p>)}
            {run.result.server_is_fork && <p>{t('resources.copyPanel.serverNowUsesOwn')}</p>}
          </Alert>
        )}
      </CardContent>
    </Card>
  )
}
