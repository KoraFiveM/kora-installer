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

/** Share a change with the original author: checks first that you have commits they do not, then opens the pull request. */
export function OpenPullRequest({ page }: { page: ResourcePage }) {
  const s = page.server.name
  const blocked = whyNot(useRole(), 'admin')
  const [title, setTitle] = useState('')
  const [text, setText] = useState('')
  const plan = useMutation({ mutationFn: () => api.feedback.prPlan(s, page.id) })
  const open = useMutation({ mutationFn: (p: PrPlan) => api.feedback.prOpen(s, { plan: p, title: title.trim(), text }) })
  return (
    <section className="space-y-2" aria-label="Open a pull request">
      <h3 className="text-sm font-semibold">Share your changes with the original author</h3>
      <p className="text-xs text-muted-foreground">A pull request asks the author to take your changes. Kora checks first that you have commits they do not.</p>
      <Button disabled={plan.isPending || !!blocked} title={blocked ?? undefined} onClick={() => plan.mutate()}>Check my changes</Button>
      {plan.data && (
        <form className="space-y-3" onSubmit={(e) => { e.preventDefault(); if (!blocked && title.trim()) open.mutate(plan.data) }}>
          <Alert tone="info" title="Ready to send">{plan.data.ahead} change{plan.data.ahead === 1 ? '' : 's'} from {plan.data.head_owner}:{plan.data.head_branch} to {plan.data.upstream} ({plan.data.base}).</Alert>
          <Field label="Title"><Input value={title} onChange={(e) => setTitle(e.target.value)} /></Field>
          <Field label="Description"><Textarea value={text} onChange={(e) => setText(e.target.value)} /></Field>
          <Button type="submit" variant="default" disabled={!title.trim() || open.isPending || !!blocked} title={blocked ?? undefined}>Open pull request</Button>
          <ActingAs />
        </form>
      )}
      {open.data && <Alert tone="success" title="Pull request opened"><a className="break-all text-info underline" href={open.data.url} target="_blank" rel="noopener noreferrer">{open.data.url}</a></Alert>}
      <ErrorNotice error={plan.error ?? open.error} />
    </section>
  )
}
