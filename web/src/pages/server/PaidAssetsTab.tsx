import { useRef, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { ExternalLink, FileArchive, Upload } from 'lucide-react'
import { api } from '@/api/client'
import type { AssetHint, AssetImportEvent, AssetImportReport, AssetInspectResult, AssetPlan, AssetSource, PaidAsset, ServerDetail } from '@/api/types'
import { useJob } from '@/hooks/useJob'
import { useRole, whyNot } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { ErrorNotice } from '@/components/ErrorNotice'
import { timeAgo } from '@/lib/ago'

export const PORTAL_URL = 'https://portal.cfx.re/assets/granted-assets'

/** `1.5 MB`-style size. */
export function fileSize(n: number): string {
  const units = ['bytes', 'KB', 'MB', 'GB']
  let v = n
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  return i === 0 ? `${n} bytes` : `${v.toFixed(1)} ${units[i]}`
}

const HINT_TITLE: Record<AssetHint['kind'], string> = {
  entitlement: 'Licence key and asset owner do not match',
  verify_failed: 'A protected resource could not be verified',
  download_failed: 'A protected resource could not be fetched',
}

function Hints({ hints }: { hints: AssetHint[] }) {
  if (hints.length === 0) return null
  return (
    <div className="space-y-3">
      {hints.map((h, i) => (
        <Alert key={`${h.kind}-${h.resource ?? i}`} tone="warning" title={`${HINT_TITLE[h.kind]}${h.resource ? `: ${h.resource}` : ''}`}>
          <p>{h.fix}</p>
          <p className="mt-1 break-all font-mono text-xs text-muted-foreground">Seen in the console: {h.line}</p>
        </Alert>
      ))}
    </div>
  )
}

export function PlanView({ plan }: { plan: AssetPlan }) {
  const blocking = plan.problems.filter((p) => p.blocking)
  const warnings = plan.problems.filter((p) => !p.blocking)
  return (
    <div className="space-y-3" aria-label="What will be imported">
      <div className="flex items-center gap-2 text-sm font-semibold"><FileArchive className="size-4" /> {plan.zip_name}</div>
      {plan.resources.length > 0 && (
        <ul className="divide-y divide-border rounded-md border border-border">
          {plan.resources.map((r) => (
            <li key={r.zip_path || r.name} className="flex flex-wrap items-center gap-2 p-3 text-sm">
              <span className="font-mono text-xs">{r.name}</span>
              {!r.has_fxmanifest ? <Badge tone="plain">Not a resource, skipped</Badge> : (
                <>
                  {r.version && <Badge tone="plain">Version {r.version}</Badge>}
                  {r.escrowed ? <Badge tone="info">Protected (escrowed)</Badge> : <Badge tone="plain">Not protected</Badge>}
                  {r.escrowed && (r.has_fxap ? <Badge tone="success">.fxap file present</Badge> : <Badge tone="danger">.fxap file missing</Badge>)}
                  {r.replaces && <Badge tone="amber">Replaces the imported copy (kept as a backup)</Badge>}
                  <span className="ml-auto font-mono text-xs text-muted-foreground">resources/[paid]/{r.name} · {fileSize(r.size)}</span>
                </>
              )}
            </li>
          ))}
        </ul>
      )}
      {blocking.length > 0 && (
        <Alert tone="danger" title="This zip can't be imported">
          <ul className="list-disc pl-5">{blocking.map((p, i) => <li key={i}>{p.message}</li>)}</ul>
        </Alert>
      )}
      {warnings.length > 0 && (
        <Alert tone="warning" title="Check before importing">
          <ul className="list-disc pl-5">{warnings.map((p, i) => <li key={i}>{p.message}</li>)}</ul>
        </Alert>
      )}
    </div>
  )
}

function sourceOf(r: AssetInspectResult): AssetSource | null {
  if (r.upload) return { upload: r.upload }
  if (r.download) return { download: r.download }
  return null
}

function ImportPanel({ server: s, canImport, local, onDone, fileInput }: { server: ServerDetail; canImport: boolean; local: boolean; onDone: () => void; fileInput: React.RefObject<HTMLInputElement | null> }) {
  const [inspected, setInspected] = useState<AssetInspectResult | null>(null)
  const [dragging, setDragging] = useState(false)
  const [job, setJob] = useState<string | null>(null)
  const [err, setErr] = useState<unknown>(null)
  const downloads = useQuery({ queryKey: ['asset-downloads'], queryFn: api.assets.downloads, enabled: local && canImport, retry: false })
  const run = useJob<AssetImportReport, AssetImportEvent>(job, onDone)
  const inspect = useMutation({
    mutationFn: (what: { file: File } | { download: string }) => ('file' in what ? api.assets.inspectUpload(s.name, what.file, what.file.name) : api.assets.inspectDownload(s.name, what.download)),
    onMutate: () => { setInspected(null); setErr(null); setJob(null) },
    onSuccess: (r) => setInspected(r),
  })
  const pick = (files: FileList | null) => {
    const f = files?.[0]
    if (!f) return
    if (!f.name.toLowerCase().endsWith('.zip')) {
      setErr(new Error('Choose the .zip file you downloaded from the Cfx.re Portal.'))
      return
    }
    inspect.mutate({ file: f })
  }
  const source = inspected ? sourceOf(inspected) : null
  const count = inspected?.plan.resources.filter((r) => r.has_fxmanifest).length ?? 0
  const startImport = async () => {
    if (!source) return
    setErr(null)
    try {
      setJob((await api.assets.import(s.name, source)).job)
    } catch (e) {
      setErr(e)
    }
  }
  const status = run.events.filter((e): e is Extract<AssetImportEvent, { type: 'status' }> => e.type === 'status').map((e) => e.message)
  return (
    <div className="space-y-4">
      <div
        role="group"
        aria-label="Drop a zip file here"
        className={`flex flex-col items-center gap-3 rounded-md border-2 border-dashed p-6 text-center text-sm ${dragging ? 'border-primary bg-primary/5' : 'border-border'}`}
        onDragOver={(e) => { e.preventDefault(); if (canImport) setDragging(true) }}
        onDragLeave={() => setDragging(false)}
        onDrop={(e) => { e.preventDefault(); setDragging(false); if (canImport) pick(e.dataTransfer.files) }}
      >
        <Upload className="size-6 text-muted-foreground" />
        <p>Drop the asset's .zip file here, exactly as you downloaded it (don't unpack it).</p>
        <input ref={fileInput} id="asset-zip" type="file" accept=".zip,application/zip" className="sr-only" disabled={!canImport} onChange={(e) => { pick(e.target.files); e.target.value = '' }} />
        <Button asChild variant="secondary" size="sm"><label htmlFor="asset-zip" aria-disabled={!canImport} className={canImport ? 'cursor-pointer' : 'pointer-events-none opacity-50'}>Choose a zip file</label></Button>
      </div>
      {local && canImport && downloads.data && (
        <div className="space-y-2">
          <div className="text-sm font-semibold">Recent zips in your Downloads folder</div>
          {downloads.data.zips.length === 0 ? (
            <p className="text-sm text-muted-foreground">{downloads.data.found ? 'No zip files were downloaded in the last 24 hours.' : 'No Downloads folder was found on this computer.'}</p>
          ) : (
            <ul className="divide-y divide-border rounded-md border border-border">
              {downloads.data.zips.map((z) => (
                <li key={z.name} className="flex flex-wrap items-center gap-2 p-2 text-sm">
                  <span className="font-mono text-xs">{z.name}</span>
                  <span className="text-xs text-muted-foreground">{fileSize(z.size)} · {timeAgo(z.modified_unix * 1000, Date.now())}</span>
                  <Button size="sm" className="ml-auto" disabled={inspect.isPending} onClick={() => inspect.mutate({ download: z.name })}>Check this zip</Button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
      {inspect.isPending && <p className="text-sm text-muted-foreground">Reading the zip…</p>}
      <ErrorNotice error={err ?? inspect.error ?? (run.phase === 'failed' ? run.error : null)} />
      {inspected && <PlanView plan={inspected.plan} />}
      {inspected?.plan.can_import && source && run.phase !== 'done' && (
        <div className="flex flex-wrap gap-2">
          <Button variant="default" disabled={run.phase === 'running'} onClick={() => void startImport()}>Import {count} asset{count === 1 ? '' : 's'}</Button>
          <Button disabled={run.phase === 'running'} onClick={() => setInspected(null)}>Cancel</Button>
        </div>
      )}
      {run.phase === 'running' && <p className="text-sm text-muted-foreground">{status.at(-1) ?? 'Importing…'}</p>}
      {run.phase === 'done' && run.result && (
        <Alert tone="success" title="Imported">
          <ul className="list-disc pl-5">
            {run.result.imported.map((a) => <li key={a.name}><span className="font-mono">{a.name}</span>{a.version ? ` ${a.version}` : ''}{a.replaced ? ' (replaced; the old copy is kept as a backup)' : ''}</li>)}
          </ul>
          {run.result.ensured.length > 0 && <p>Added to server.cfg: {run.result.ensured.map((e) => `ensure ${e}`).join(', ')}</p>}
          {run.result.notes.map((n, i) => <p key={i}>{n}</p>)}
        </Alert>
      )}
    </div>
  )
}

function AssetRow({ server, asset: a, canChange, onReplace, onRemoved }: { server: string; asset: PaidAsset; canChange: boolean; onReplace: () => void; onRemoved: (notes: string[]) => void }) {
  const [confirm, setConfirm] = useState(false)
  const m = useMutation({ mutationFn: () => api.assets.remove(server, a.name), onSuccess: (r) => { setConfirm(false); onRemoved(r.notes) } })
  return (
    <li className="space-y-2 p-3 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-mono text-xs">{a.name}</span>
        {a.version && <Badge tone="plain">Version {a.version}</Badge>}
        {a.imported_unix && <span className="text-xs text-muted-foreground">Imported {new Date(a.imported_unix * 1000).toLocaleDateString()}</span>}
        {!a.present && <Badge tone="danger">Folder missing</Badge>}
        {a.present && !a.has_fxap && <Badge tone="amber">No .fxap file</Badge>}
        {canChange && (
          <span className="ml-auto flex gap-2">
            <Button size="sm" onClick={onReplace}>Replace with newer zip</Button>
            <Button size="sm" variant="outline" onClick={() => setConfirm(true)}>Remove</Button>
          </span>
        )}
      </div>
      <ErrorNotice error={m.error} />
      <ConfirmDialog
        open={confirm}
        onOpenChange={setConfirm}
        title={`Remove ${a.name}?`}
        description="Its folder is moved to the backups in the server's .kora folder and its ensure line is taken out of server.cfg. Restart the server afterwards."
        confirmLabel="Remove"
        destructive
        busy={m.isPending}
        onConfirm={() => m.mutate()}
      />
    </li>
  )
}

export function PaidAssetsTab({ server: s }: { server: ServerDetail }) {
  const qc = useQueryClient()
  const role = useRole()
  const canImport = role.can('admin')
  const noList = whyNot(role, 'operator')
  const local = !role.remote
  const fileInput = useRef<HTMLInputElement>(null)
  const [removed, setRemoved] = useState<string[] | null>(null)
  const list = useQuery({ queryKey: ['paid-assets', s.name], queryFn: () => api.assets.list(s.name), enabled: !noList })
  const refresh = () => {
    void qc.invalidateQueries({ queryKey: ['paid-assets', s.name] })
    void qc.invalidateQueries({ queryKey: ['resources', s.name] })
  }
  const portal = list.data?.portal_url ?? PORTAL_URL
  const assets = list.data?.assets ?? []
  return (
    <div className="grid gap-5 lg:grid-cols-2">
      <div className="space-y-5">
        <Card>
          <CardHeader><CardTitle>Paid assets</CardTitle></CardHeader>
          <CardContent className="space-y-3 text-sm">
            <p>Assets you buy on the Cfx.re Portal are protected (escrowed). Download each one as a .zip file and give it to Kora: it keeps the folder name and the hidden .fxap file exactly as they must be, puts the asset in resources/[paid] and adds it to server.cfg.</p>
            <Alert tone="info">Buy and download with the same Cfx.re account that owns this server's licence key.</Alert>
            <Button asChild variant="default">
              <a href={portal} target="_blank" rel="noopener noreferrer">Open Cfx.re Portal <ExternalLink /></a>
            </Button>
          </CardContent>
        </Card>
        <Hints hints={list.data?.hints ?? []} />
        <Card>
          <CardHeader><CardTitle>Imported paid assets{noList ? '' : ` (${assets.length})`}</CardTitle></CardHeader>
          <CardContent className="space-y-3">
            <ErrorNotice error={list.error} />
            {removed && <Alert tone="success" title="Removed">{removed.map((n, i) => <p key={i}>{n}</p>)}</Alert>}
            {noList ? <Alert tone="info">{noList}</Alert> : assets.length === 0 && !list.isLoading ? <p className="text-sm text-muted-foreground">No paid assets have been imported yet.</p> : (
              <ul className="divide-y divide-border rounded-md border border-border">
                {assets.map((a) => <AssetRow key={a.name} server={s.name} asset={a} canChange={canImport} onReplace={() => fileInput.current?.click()} onRemoved={(n) => { setRemoved(n); refresh() }} />)}
              </ul>
            )}
          </CardContent>
        </Card>
      </div>
      <Card>
        <CardHeader><CardTitle>Import a paid asset</CardTitle></CardHeader>
        <CardContent className="space-y-3">
          {!canImport && <Alert tone="info">{whyNot(role, 'admin')}</Alert>}
          <ImportPanel server={s} canImport={canImport} local={local} onDone={refresh} fileInput={fileInput} />
        </CardContent>
      </Card>
    </div>
  )
}
