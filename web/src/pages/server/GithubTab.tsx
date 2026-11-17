import { useState } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { PrPlan, ServerDetail } from '@/api/types'
import { useRole, whyNot } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
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
  const [attach, setAttach] = useState(false)
  const summary = useQuery({ queryKey: ['support-summary', s.name], queryFn: () => api.support.summary(s.name), enabled: attach, staleTime: 60_000 })
  const m = useMutation({
    mutationFn: async () => {
      // The summary is cleaned by Kora (the same removal of secrets as a support bundle); the bundle file itself is never attached.
      const extra = attach ? (summary.data ?? (await api.support.summary(s.name))).text : ''
      return api.feedback.report(s.name, { resource, title: title.trim(), text: extra ? `${text}\n\n${extra}` : text })
    },
  })
  return (
    <form className="space-y-3" onSubmit={(e) => { e.preventDefault(); if (!blocked) m.mutate() }}>
      <Field label="Resource"><Select value={resource} onChange={(e) => setResource(e.target.value)}><option value="">Choose a resource</option>{resources.map((r) => <option key={r}>{r}</option>)}</Select></Field>
      <Field label="Title"><Input value={title} onChange={(e) => setTitle(e.target.value)} /></Field>
      <Field label="What happened?"><Textarea value={text} onChange={(e) => setText(e.target.value)} /></Field>
      <Checkbox checked={attach} onChange={(e) => setAttach(e.target.checked)} label="Add a short summary of this computer" hint="Your Kora version, system and tool versions, with secrets removed. The support bundle file is not attached. Issues are public." />
      {attach && summary.data && <details><summary className="cursor-pointer text-xs text-muted-foreground">Show what will be added</summary><pre aria-label="Summary that will be added" className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded-md border border-border bg-ink p-2 font-mono text-xs">{summary.data.text}</pre></details>}
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
