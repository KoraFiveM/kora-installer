import { useMemo, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { Loader2, RefreshCw } from 'lucide-react'
import { api } from '@/api/client'
import type { BranchChoices, ChangesChoice, ResourceBranch, ServerDetail, SwitchRequest, SwitchResult, Versions } from '@/api/types'
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
import { t } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/resources/resourceDialog'

/** After files changed: say that the running server still has the old version, and offer the existing restart. */
/** What the branch tools need to know about the server. */
export type BranchServer = Pick<ServerDetail, 'name' | 'running' | 'console_enabled'> & { versions: Versions }

export function RestartHint({ server: s, resource, note }: { server: Pick<ServerDetail, 'name' | 'running' | 'console_enabled'>; resource: string; note?: string }) {
  const m = useMutation({ mutationFn: () => api.resources.control(s.name, resource, 'restart') })
  const canRestart = s.running && s.console_enabled
  return (
    <div className="mt-2 space-y-2 text-sm">
      <p>{note ?? t('resources.resourceDialog.filesChangedRestartResource')}</p>
      {s.running ? (
        canRestart ? (
          <Button size="sm" onClick={() => m.mutate()} disabled={m.isPending}>{rich('resources.resourceDialog.nodeNodeRestartResource', { node: m.isPending && <Loader2 className="animate-spin" aria-hidden />, resource })}</Button>
        ) : <p className="text-xs text-muted-foreground">{t('resources.resourceDialog.restartJustResourceTurn')}</p>
      ) : <p className="text-xs text-muted-foreground">{t('resources.resourceDialog.serverRunningUseNew')}</p>}
      {m.data && <p className="text-xs text-muted-foreground" role="status">{m.data.message}</p>}
      <ErrorNotice error={m.error} />
    </div>
  )
}

/** The Branches dialog: the same controls as on the resource page, in a box. */
export function ResourceDialog({ server: s, resource, onClose }: { server: BranchServer; resource: ResourceBranch; onClose: () => void }) {
  return (
    <Dialog open onOpenChange={(o) => { if (!o) onClose() }}>
      <DialogContent className="max-w-2xl" title={t('resources.resourceDialog.branchId', { id: resource.id })} description={t('resources.resourceDialog.tryAnotherVersionResources')}>
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
  const [prText, setPrText] = useState('')
  const [result, setResult] = useState<SwitchResult | null>(null)
  const [ask, setAsk] = useState<{ req: SwitchRequest; files: string[] } | null>(null)

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
            <Alert tone="warning" title={t('resources.resourceDialog.serverFollowsReleasedVersions')}>
              <p>{t('resources.resourceDialog.resourceRunUnreleasedCode')}</p>
              <div className="mt-2"><Checkbox label={t('resources.resourceDialog.iUnderstandResourceRun')} checked={understood} onChange={(e) => setUnderstood(e.target.checked)} /></div>
            </Alert>
          )}
          <div className="flex flex-wrap items-center gap-2 text-sm">
            <span>{t('resources.resourceDialog.rightNow')}</span>
            <VersionBadge branch={c?.resource ?? resource} />
            {(c?.resource ?? resource).changed_count > 0 && <Badge tone="amber">{t('resources.resourceDialog.changedCountChangedFile', { changed_count: (c?.resource ?? resource).changed_count })}</Badge>}
          </div>
          {blocked && <p className="text-xs text-muted-foreground">{blocked}</p>}
          <ErrorNotice error={choices.error} onRetry={() => void choices.refetch()} />

          {result && (
            <Alert tone={result.status === 'already' ? 'info' : 'success'} title={result.status === 'already' ? t('resources.resourceDialog.nothingDo') : t('resources.resourceDialog.done')}>
              <p>{result.message}</p>
              {result.stashed && <p className="mt-1">{t('resources.resourceDialog.changesKeptAsideMessage', { message: result.stashed.message })}</p>}
              {result.saved_branch && <p className="mt-1">{rich('resources.resourceDialog.commitWereHadNo', { code: <code className="font-mono">{result.saved_branch}</code> })}</p>}
              {result.needs_restart && <RestartHint server={s} resource={id} note={result.restart_note} />}
            </Alert>
          )}
          <ErrorNotice error={go.error ?? restore.error} onRetry={go.variables ? () => go.mutate(go.variables) : undefined} />

          {resource.state !== 'default' && (
            <section className="space-y-2">
              <h3 className="text-sm font-semibold">{released ? t('resources.resourceDialog.backReleasedVersion') : t('resources.resourceDialog.backNormalVersion')}</h3>
              <p className="text-xs text-muted-foreground">{released ? t('resources.resourceDialog.goesBackExactlyRelease') : <>{rich('resources.resourceDialog.goesBackCodeNode', { code: <code className="font-mono">{resource.default_branch}</code> })}</>}</p>
              <Button disabled={working || !!blocked} onClick={() => send({ action: 'default' })}>{released ? t('resources.resourceDialog.backReleasedVersion') : t('resources.resourceDialog.backNormalVersion')}</Button>
            </section>
          )}

          <section className="space-y-2">
            <h3 className="text-sm font-semibold">{t('resources.resourceDialog.switchBranch')}</h3>
            <p className="text-xs text-muted-foreground">{t('resources.resourceDialog.branchSeparateLineChanges')}</p>
            <div className="flex flex-wrap items-end gap-2">
              <div className="min-w-56 flex-1"><Field label={t('resources.resourceDialog.branch')}>
                <Select value={pick} onChange={(e) => setPick(e.target.value)} disabled={!c}>
                  <option value="">{t('resources.resourceDialog.chooseBranch')}</option>
                  {options.local.length > 0 && <optgroup label={t('resources.resourceDialog.alreadyFolder')}>{options.local.map((b) => <option key={`l${b}`} value={`local:${b}`}>{b}</option>)}</optgroup>}
                  {options.fork.length > 0 && <optgroup label={t('resources.resourceDialog.onYourFork')}>{options.fork.map((b) => <option key={`f${b}`} value={`origin:${b}`}>{b}</option>)}</optgroup>}
                  {options.upstream.length > 0 && <optgroup label={t('resources.resourceDialog.originalRepository')}>{options.upstream.map((b) => <option key={`u${b}`} value={`upstream:${b}`}>{b}</option>)}</optgroup>}
                </Select>
              </Field></div>
              <Button variant="default" disabled={!pick || working || leaving} onClick={switchToPick}>{t('resources.resourceDialog.switch')}</Button>
              <Button variant="ghost" onClick={() => reload.mutate()} disabled={reload.isPending} title={t('resources.resourceDialog.askGithubNewestList')} aria-label={t('resources.resourceDialog.refreshListBranchesFrom')}>{rich('resources.resourceDialog.nodeNodeRefreshList', { node: reload.isPending ? <Loader2 className="animate-spin" aria-hidden /> : <RefreshCw aria-hidden /> })}</Button>
            </div>
            {(c?.notes ?? []).map((n) => <p key={n} className="text-xs text-amber">{n}</p>)}
            <ErrorNotice error={reload.error} />
          </section>

          <section className="space-y-2">
            <h3 className="text-sm font-semibold">{t('resources.resourceDialog.tryPullRequest')}</h3>
            <p className="text-xs text-muted-foreground">{t('resources.resourceDialog.pullRequestChangeSomeone')}</p>
            <p className="text-xs text-amber" role="note">{t('resources.resourceDialog.pullRequestSomeoneElses')}</p>
            <form className="flex flex-wrap items-end gap-2" onSubmit={(e) => { e.preventDefault(); if (prText.trim() && !leaving) send({ action: 'pr', pr: prText.trim() }) }}>
              <div className="min-w-56 flex-1"><Field label={t('resources.resourceDialog.pullRequestNumberAddress')}><Input value={prText} onChange={(e) => setPrText(e.target.value)} placeholder={t('resources.resourceDialog.httpsGithubComOwner')} autoComplete="off" /></Field></div>
              <Button type="submit" variant="default" disabled={!prText.trim() || working || leaving}>{t('resources.resourceDialog.tryIt')}</Button>
            </form>
            <p className="text-xs text-muted-foreground">{rich('resources.resourceDialog.pickOneFromOpen', { id, link: (c) => <Link className="text-info underline" to={`/servers/${encodeURIComponent(s.name)}/resources/${encodeURIComponent(id)}`}>{c}</Link> })}</p>
          </section>

          {(c?.stashes.length ?? 0) > 0 && (
            <section className="space-y-2">
              <h3 className="text-sm font-semibold">{t('resources.resourceDialog.changesKeptAside')}</h3>
              <p className="text-xs text-muted-foreground">{t('resources.resourceDialog.koraSetTheseAside')}</p>
              <ul className="divide-y divide-border rounded-md border border-border">
                {c!.stashes.map((st) => (
                  <li key={st.index} className="flex flex-wrap items-center gap-2 p-3 text-sm">
                    <span className="min-w-0 flex-1">{st.message} <span className="text-xs text-muted-foreground">{t('resources.resourceDialog.madeBranch', { branch: st.branch })}</span></span>
                    <Button size="sm" disabled={working || !!blocked} aria-label={t('resources.resourceDialog.bringMyChangesBack', { value: st.index + 1 })} onClick={() => restore.mutate(st.index)}>{t('resources.resourceDialog.bringMyChangesBack2')}</Button>
                  </li>
                ))}
              </ul>
              {resource.changed_count > 0 && <p className="text-xs text-muted-foreground">{t('resources.resourceDialog.bringingChangesBackNeeds')}</p>}
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
