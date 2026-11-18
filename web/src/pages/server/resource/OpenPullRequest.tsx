import { useState } from 'react'
import { useMutation } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { PrPlan, ResourcePage } from '@/api/types'
import { useRole, whyNot } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input, Textarea } from '@/components/ui/input'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { ActingAs } from './ActingAs'
import { t } from '@/i18n'
import '@/i18n/en/resources/openPullRequest'

/** Share a change with the original author: checks first that you have commits they do not, then opens the pull request. */
export function OpenPullRequest({ page }: { page: ResourcePage }) {
  const s = page.server.name
  const blocked = whyNot(useRole(), 'admin')
  const [title, setTitle] = useState('')
  const [text, setText] = useState('')
  const plan = useMutation({ mutationFn: () => api.feedback.prPlan(s, page.id) })
  const open = useMutation({ mutationFn: (p: PrPlan) => api.feedback.prOpen(s, { plan: p, title: title.trim(), text }) })
  return (
    <section className="space-y-2" aria-label={t('resources.openPullRequest.openPullRequest')}>
      <h3 className="text-sm font-semibold">{t('resources.openPullRequest.shareChangesOriginalAuthor')}</h3>
      <p className="text-xs text-muted-foreground">{t('resources.openPullRequest.pullRequestAsksAuthor')}</p>
      <Button disabled={plan.isPending || !!blocked} title={blocked ?? undefined} onClick={() => plan.mutate()}>{t('resources.openPullRequest.checkMyChanges')}</Button>
      {plan.data && (
        <form className="space-y-3" onSubmit={(e) => { e.preventDefault(); if (!blocked && title.trim()) open.mutate(plan.data) }}>
          <Alert tone="info" title={t('resources.openPullRequest.readySend')}>{t('resources.openPullRequest.aheadChangeValueFrom', { ahead: plan.data.ahead, head_owner: plan.data.head_owner, head_branch: plan.data.head_branch, upstream: plan.data.upstream, base: plan.data.base })}</Alert>
          <Field label={t('resources.openPullRequest.title')}><Input value={title} onChange={(e) => setTitle(e.target.value)} /></Field>
          <Field label={t('resources.openPullRequest.description')}><Textarea value={text} onChange={(e) => setText(e.target.value)} /></Field>
          <Button type="submit" variant="default" disabled={!title.trim() || open.isPending || !!blocked} title={blocked ?? undefined}>{t('resources.openPullRequest.openPullRequest2')}</Button>
          <ActingAs />
        </form>
      )}
      {open.data && <Alert tone="success" title={t('resources.openPullRequest.pullRequestOpened')}><a className="break-all text-info underline" href={open.data.url} target="_blank" rel="noopener noreferrer">{open.data.url}</a></Alert>}
      <ErrorNotice error={plan.error ?? open.error} />
    </section>
  )
}
