import { useMemo, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Loader2, RefreshCw } from 'lucide-react'
import { api } from '@/api/client'
import type { BranchChoices, ChangesChoice, PullRequest, ResourceBranch, ServerDetail, SwitchRequest, SwitchResult, Versions } from '@/api/types'
import { useRole, whyNot } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent } from '@/components/ui/dialog'
import { Checkbox } from '@/components/ui/checkbox'
import { Input, Select } from '@/components/ui/input'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { useRefreshBranches } from './useBranches'
import { VersionBadge } from './VersionBadge'
import { ChangesDialog } from './ChangesDialog'

/** After files changed: say that the running server still has the old version, and offer the existing restart. */
/** What the branch tools need to know about the server. */
export type BranchServer = Pick<ServerDetail, 'name' | 'running' | 'console_enabled'> & { versions: Versions }

export function RestartHint({ server: s, resource, note }: { server: Pick<ServerDetail, 'name' | 'running' | 'console_enabled'>; resource: string; note?: string }) {
  const m = useMutation({ mutationFn: () => api.resources.control(s.name, resource, 'restart') })
  const canRestart = s.running && s.console_enabled
  return (
    <div className="mt-2 space-y-2 text-sm">
      <p>{note ?? 'The files changed. Restart the resource (or the server) to load them.'}</p>
      {s.running ? (
        canRestart ? (
          <Button size="sm" onClick={() => m.mutate()} disabled={m.isPending}>{m.isPending && <Loader2 className="animate-spin" aria-hidden />}Restart {resource} now</Button>
        ) : <p className="text-xs text-muted-foreground">To restart just this resource, turn on the live console in the Console tab. Or restart the whole server from the Overview tab.</p>
      ) : <p className="text-xs text-muted-foreground">The server is not running, so it will use the new version when it starts.</p>}
      {m.data && <p className="text-xs text-muted-foreground" role="status">{m.data.message}</p>}
      <ErrorNotice error={m.error} />
    </div>
  )
}

const prWhere = (p: PullRequest) => (p.source === 'upstream' ? 'Original repository' : 'Your fork')

/** The Branches dialog: the same controls as on the resource page, in a box. */
export function ResourceDialog({ server: s, resource, onClose }: { server: BranchServer; resource: ResourceBranch; onClose: () => void }) {
  return (
    <Dialog open onOpenChange={(o) => { if (!o) onClose() }}>
      <DialogContent className="max-w-2xl" title={`Branch of ${resource.id}`} description="You can try another version of this resource's code, or go back to the normal one.">
        <BranchControls server={s} resource={resource} />
      </DialogContent>
    </Dialog>
  )
}

/** Where the resource is and how to change it: back to normal, another branch, a pull request, kept-aside changes. */
export function BranchControls({ server: s, resource }: { server: BranchServer; resource: ResourceBranch }) {
  const qc = useQueryClient()
  const refreshLists = useRefreshBranches(s.name)
  const role = useRole()
  const blocked = whyNot(role, 'admin')
  const id = resource.id
  const choices = useQuery({ queryKey: ['branch-choices', s.name, id], queryFn: () => api.branches.choices(s.name, id) })
  const [pick, setPick] = useState('')
  const [showPrs, setShowPrs] = useState(false)
  const [prText, setPrText] = useState('')
  const [result, setResult] = useState<SwitchResult | null>(null)
  const [ask, setAsk] = useState<{ req: SwitchRequest; files: string[] } | null>(null)
  const prs = useQuery({ queryKey: ['branch-prs', s.name, id], queryFn: () => api.branches.prs(s.name, id), enabled: showPrs, retry: false })

  const reload = useMutation({
    mutationFn: () => api.branches.choices(s.name, id, true),
    onSuccess: (c) => qc.setQueryData<BranchChoices>(['branch-choices', s.name, id], c),
  })
  const done = (r: SwitchResult) => {
    setResult(r)
    refreshLists()
  }
  const go = useMutation({
    mutationFn: (req: SwitchRequest) => api.branches.switch(s.name, id, req).then((r) => ({ r, req })),
    onSuccess: ({ r, req }) => {
      if (r.status === 'needs_choice') {
        setResult(null)
        setAsk({ req, files: r.now.changed_files })
      } else done(r)
    },
  })
  const restore = useMutation({ mutationFn: (index: number) => api.branches.restore(s.name, id, index), onSuccess: done })

  const c = choices.data
  const options = useMemo(() => {
    if (!c) return { local: [], fork: [], upstream: [] }
    const local = c.branches.local.filter((b) => b !== c.branches.current)
    const fork = c.branches.fork.filter((b) => !c.branches.local.includes(b))
    const upstream = c.branches.upstream.filter((b) => !c.branches.local.includes(b) && !c.branches.fork.includes(b))
    return { local, fork, upstream }
  }, [c])
  const send = (req: SwitchRequest) => { setResult(null); go.mutate(req) }
  const switchToPick = () => {
    const [where, ...rest] = pick.split(':')
    const branch = rest.join(':')
    send({ action: 'branch', branch, ...(where === 'origin' || where === 'upstream' ? { remote: where } : {}) })
  }
  const working = go.isPending || restore.isPending

  const released = s.versions === 'released'
  // A server on released versions runs tested code; leaving the release is a deliberate choice.
  const [understood, setUnderstood] = useState(false)
  const leaving = !!blocked || (released && !understood)
  return (
    <>
        <div className="space-y-5">
          {released && (
            <Alert tone="warning" title="This server follows released versions">
              <p>This resource will run unreleased code until you put it back on its released version. Only do it for a test, and put it back when you are done.</p>
              <div className="mt-2"><Checkbox label="I understand: this resource will run unreleased code" checked={understood} onChange={(e) => setUnderstood(e.target.checked)} /></div>
            </Alert>
          )}
          <div className="flex flex-wrap items-center gap-2 text-sm">
            <span>Right now it is on</span>
            <VersionBadge branch={c?.resource ?? resource} />
            {(c?.resource ?? resource).changed_count > 0 && <Badge tone="amber">{(c?.resource ?? resource).changed_count} changed file{(c?.resource ?? resource).changed_count === 1 ? '' : 's'} of yours</Badge>}
          </div>
          {blocked && <p className="text-xs text-muted-foreground">{blocked}</p>}
          <ErrorNotice error={choices.error} onRetry={() => void choices.refetch()} />

          {result && (
            <Alert tone={result.status === 'already' ? 'info' : 'success'} title={result.status === 'already' ? 'Nothing to do' : 'Done'}>
              <p>{result.message}</p>
              {result.stashed && <p className="mt-1">Your changes are kept aside ({result.stashed.message}). You can bring them back below.</p>}
              {result.saved_branch && <p className="mt-1">The commit you were on had no branch, so it was saved as <code className="font-mono">{result.saved_branch}</code>.</p>}
              {result.needs_restart && <RestartHint server={s} resource={id} note={result.restart_note} />}
            </Alert>
          )}
          <ErrorNotice error={go.error ?? restore.error} onRetry={go.variables ? () => go.mutate(go.variables) : undefined} />

          {resource.state !== 'default' && (
            <section className="space-y-2">
              <h3 className="text-sm font-semibold">{released ? 'Back to the released version' : 'Back to the normal version'}</h3>
              <p className="text-xs text-muted-foreground">{released ? 'Goes back to exactly the release this server is pinned to. Updates start working again for this resource.' : <>Goes back to <code className="font-mono">{resource.default_branch}</code>, the branch Kora keeps updated. Updates start working again for this resource.</>}</p>
              <Button disabled={working || !!blocked} onClick={() => send({ action: 'default' })}>{released ? 'Back to the released version' : 'Back to the normal version'}</Button>
            </section>
          )}

          <section className="space-y-2">
            <h3 className="text-sm font-semibold">Switch to a branch</h3>
            <p className="text-xs text-muted-foreground">A branch is a separate line of changes to the code. Pick one from your fork or from the original repository.</p>
            <div className="flex flex-wrap items-end gap-2">
              <div className="min-w-56 flex-1"><Field label="Branch">
                <Select value={pick} onChange={(e) => setPick(e.target.value)} disabled={!c}>
                  <option value="">Choose a branch</option>
                  {options.local.length > 0 && <optgroup label="Already in this folder">{options.local.map((b) => <option key={`l${b}`} value={`local:${b}`}>{b}</option>)}</optgroup>}
                  {options.fork.length > 0 && <optgroup label="On your fork">{options.fork.map((b) => <option key={`f${b}`} value={`origin:${b}`}>{b}</option>)}</optgroup>}
                  {options.upstream.length > 0 && <optgroup label="On the original repository">{options.upstream.map((b) => <option key={`u${b}`} value={`upstream:${b}`}>{b}</option>)}</optgroup>}
                </Select>
              </Field></div>
              <Button variant="default" disabled={!pick || working || leaving} onClick={switchToPick}>Switch</Button>
              <Button variant="ghost" onClick={() => reload.mutate()} disabled={reload.isPending} title="Ask GitHub for the newest list of branches" aria-label="Refresh the branch list from GitHub">
                {reload.isPending ? <Loader2 className="animate-spin" aria-hidden /> : <RefreshCw aria-hidden />}Refresh list
              </Button>
            </div>
            {(c?.notes ?? []).map((n) => <p key={n} className="text-xs text-amber">{n}</p>)}
            <ErrorNotice error={reload.error} />
          </section>

          <section className="space-y-2">
            <h3 className="text-sm font-semibold">Try a pull request</h3>
            <p className="text-xs text-muted-foreground">A pull request is a change someone has proposed. Kora downloads it so you can try it on your server before it is accepted. Paste its number or address, or pick one.</p>
            <p className="text-xs text-amber" role="note">A pull request is someone else&apos;s code. Once you try it, it runs on your server like any of its resources, before anyone has accepted it. Only try pull requests from people you trust, and look at what they change on GitHub first.</p>
            <form className="flex flex-wrap items-end gap-2" onSubmit={(e) => { e.preventDefault(); if (prText.trim() && !leaving) send({ action: 'pr', pr: prText.trim() }) }}>
              <div className="min-w-56 flex-1"><Field label="Pull request number or address"><Input value={prText} onChange={(e) => setPrText(e.target.value)} placeholder="12 or https://github.com/owner/name/pull/12" autoComplete="off" /></Field></div>
              <Button type="submit" variant="default" disabled={!prText.trim() || working || leaving}>Try it</Button>
            </form>
            {!showPrs ? (
              <Button size="sm" onClick={() => setShowPrs(true)}>Show open pull requests</Button>
            ) : (
              <div className="space-y-2">
                {prs.isLoading && <p className="text-sm text-muted-foreground" role="status">Asking GitHub...</p>}
                <ErrorNotice error={prs.error} onRetry={() => void prs.refetch()} />
                {(prs.data?.notes ?? []).map((n) => <p key={n} className="text-xs text-amber">{n}</p>)}
                {prs.data && prs.data.prs.length === 0 && <p className="text-sm text-muted-foreground">There are no open pull requests.</p>}
                <ul className="divide-y divide-border rounded-md border border-border" aria-label="Open pull requests">
                  {(prs.data?.prs ?? []).map((p) => (
                    <li key={`${p.source}${p.number}`} className="flex flex-wrap items-center gap-2 p-3 text-sm">
                      <div className="min-w-0 flex-1">
                        <div><span className="font-mono text-xs text-muted-foreground">#{p.number}</span> {p.title}{p.draft && <> <Badge tone="plain">Draft</Badge></>}</div>
                        <div className="mt-0.5 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                          <Badge tone={p.source === 'upstream' ? 'info' : 'amber'}>{prWhere(p)}</Badge>
                          <span>by {p.author}</span>
                          <span className="font-mono">{p.repo}</span>
                          {p.from === null && <span className="text-orange">its fork was deleted</span>}
                        </div>
                      </div>
                      <Button size="sm" disabled={working || leaving} aria-label={`Try pull request #${p.number}`} onClick={() => send({ action: 'pr', pr: p.number, source: p.source })}>Try it</Button>
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </section>

          {(c?.stashes.length ?? 0) > 0 && (
            <section className="space-y-2">
              <h3 className="text-sm font-semibold">Changes you kept aside</h3>
              <p className="text-xs text-muted-foreground">Kora set these aside when you switched. They stay here until you bring them back, and bringing them back does not delete them. They are also kept in this folder in git, as a "stash".</p>
              <ul className="divide-y divide-border rounded-md border border-border">
                {c!.stashes.map((st) => (
                  <li key={st.index} className="flex flex-wrap items-center gap-2 p-3 text-sm">
                    <span className="min-w-0 flex-1">{st.message} <span className="text-xs text-muted-foreground">(made on {st.branch})</span></span>
                    <Button size="sm" disabled={working || !!blocked} aria-label={`Bring back kept-aside changes ${st.index + 1}`} onClick={() => restore.mutate(st.index)}>Bring my changes back</Button>
                  </li>
                ))}
              </ul>
              {resource.changed_count > 0 && <p className="text-xs text-muted-foreground">Bringing changes back needs a clean folder: keep or save your current changes first.</p>}
            </section>
          )}
        </div>
        <ChangesDialog
          open={!!ask}
          resources={[id]}
          files={ask?.files ?? []}
          busy={go.isPending}
          onCancel={() => setAsk(null)}
          onChoose={(choice: Exclude<ChangesChoice, 'ask'>) => {
            const req = ask?.req
            setAsk(null)
            if (req) go.mutate({ ...req, changes: choice })
          }}
        />
    </>
  )
}
