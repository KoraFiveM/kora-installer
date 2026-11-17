import { useState } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { PrPlan, ServerDetail } from '@/api/types'
import { useRole, whyNot } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input, Select, Textarea } from '@/components/ui/input'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { GithubSignIn } from '@/components/GithubSignIn'
import { BranchesPanel } from './branches/BranchesPanel'

function UrlResult({ url, text }: { url: string; text: string }) {
  return <Alert tone="success" title={text}><a className="break-all text-info underline" href={url} target="_blank" rel="noopener noreferrer">{url}</a></Alert>
}

function ReportIssue({ server: s, resources }: { server: ServerDetail; resources: string[] }) {
  const [resource, setResource] = useState('')
  const [title, setTitle] = useState('')
  const [text, setText] = useState('')
  const blocked = whyNot(useRole(), 'admin')
  const m = useMutation({ mutationFn: () => api.feedback.report(s.name, { resource, title: title.trim(), text }) })
  return (
    <form className="space-y-3" onSubmit={(e) => { e.preventDefault(); if (!blocked) m.mutate() }}>
      <Field label="Resource"><Select value={resource} onChange={(e) => setResource(e.target.value)}><option value="">Choose a resource</option>{resources.map((r) => <option key={r}>{r}</option>)}</Select></Field>
      <Field label="Title"><Input value={title} onChange={(e) => setTitle(e.target.value)} /></Field>
      <Field label="What happened?"><Textarea value={text} onChange={(e) => setText(e.target.value)} /></Field>
      <Button type="submit" variant="default" disabled={!resource || !title.trim() || m.isPending || !!blocked} title={blocked ?? undefined}>Report issue</Button>
      {blocked && <p className="text-xs text-muted-foreground">{blocked}</p>}
      {m.data && <UrlResult url={m.data.url} text="Issue reported" />}
      <ErrorNotice error={m.error} />
    </form>
  )
}

function OpenPr({ server: s, resources }: { server: ServerDetail; resources: string[] }) {
  const [resource, setResource] = useState('')
  const [title, setTitle] = useState('')
  const [text, setText] = useState('')
  const blocked = whyNot(useRole(), 'admin')
  const plan = useMutation({ mutationFn: () => api.feedback.prPlan(s.name, resource) })
  const open = useMutation({ mutationFn: (p: PrPlan) => api.feedback.prOpen(s.name, { plan: p, title: title.trim(), text }) })
  return (
    <div className="space-y-3">
      <p className="text-sm text-muted-foreground">Share a change you made with the original author. Kora checks first that you have commits they do not.</p>
      <Field label="Resource"><Select value={resource} onChange={(e) => { setResource(e.target.value); plan.reset() }}><option value="">Choose a resource</option>{resources.map((r) => <option key={r}>{r}</option>)}</Select></Field>
      <Button disabled={!resource || plan.isPending || !!blocked} title={blocked ?? undefined} onClick={() => plan.mutate()}>Check my changes</Button>
      {blocked && <p className="text-xs text-muted-foreground">{blocked}</p>}
      {plan.data && (
        <form className="space-y-3" onSubmit={(e) => { e.preventDefault(); if (!blocked) open.mutate(plan.data) }}>
          <Alert tone="info" title="Ready to send">{plan.data.ahead} change{plan.data.ahead === 1 ? '' : 's'} from {plan.data.head_owner}:{plan.data.head_branch} to {plan.data.upstream} ({plan.data.base}).</Alert>
          <Field label="Title"><Input value={title} onChange={(e) => setTitle(e.target.value)} /></Field>
          <Field label="Description"><Textarea value={text} onChange={(e) => setText(e.target.value)} /></Field>
          <Button type="submit" variant="default" disabled={!title.trim() || open.isPending || !!blocked} title={blocked ?? undefined}>Open pull request</Button>
        </form>
      )}
      {open.data && <UrlResult url={open.data.url} text="Pull request opened" />}
      <ErrorNotice error={plan.error ?? open.error} />
    </div>
  )
}

export function GithubTab({ server: s }: { server: ServerDetail }) {
  const status = useQuery({ queryKey: ['github'], queryFn: api.github.status })
  const res = useQuery({ queryKey: ['resources', s.name], queryFn: () => api.resources.list(s.name) })
  const resources = (res.data ?? []).filter((r) => r.startsWith('kora_'))
  return (
    <div className="grid gap-5 lg:grid-cols-2">
      <Card className="lg:col-span-2"><CardHeader><CardTitle>GitHub account</CardTitle></CardHeader><CardContent><GithubSignIn /></CardContent></Card>
      <div className="lg:col-span-2"><BranchesPanel server={s} /></div>
      {status.data?.signed_in && (
        <>
          <Card><CardHeader><CardTitle>Report an issue</CardTitle></CardHeader><CardContent><ReportIssue server={s} resources={resources} /></CardContent></Card>
          <Card><CardHeader><CardTitle>Open a pull request</CardTitle></CardHeader><CardContent><OpenPr server={s} resources={resources} /></CardContent></Card>
        </>
      )}
    </div>
  )
}
