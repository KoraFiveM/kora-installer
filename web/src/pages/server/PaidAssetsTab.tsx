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
import { t, msg, type Msg, fmt, unix } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/resources/paidAssetsTab'

export const PORTAL_URL = 'https://portal.cfx.re/assets/granted-assets'

/** `1.5 MB`-style size, in the current language. */
export function fileSize(n: number): string {
  return fmt.bytes(n)
}

const HINT_TITLE: Record<AssetHint['kind'], Msg> = {
  entitlement: msg('resources.paidAssetsTab.licenceKeyAssetOwner'),
  verify_failed: msg('resources.paidAssetsTab.protectedResourceCouldVerified'),
  download_failed: msg('resources.paidAssetsTab.protectedResourceCouldFetched'),
}

function Hints({ hints }: { hints: AssetHint[] }) {
  if (hints.length === 0) return null
  return (
    <div className="space-y-3">
      {hints.map((h, i) => (
        <Alert key={`${h.kind}-${h.resource ?? i}`} tone="warning" title={`${t(HINT_TITLE[h.kind])}${h.resource ? `: ${h.resource}` : ''}`}>
          <p>{h.fix}</p>
          <p className="mt-1 break-all font-mono text-xs text-muted-foreground">{t('resources.paidAssetsTab.seenConsoleLine', { line: h.line })}</p>
        </Alert>
      ))}
    </div>
  )
}

export function PlanView({ plan }: { plan: AssetPlan }) {
  const blocking = plan.problems.filter((p) => p.blocking)
  const warnings = plan.problems.filter((p) => !p.blocking)
  return (
    <div className="space-y-3" aria-label={t('resources.paidAssetsTab.whatImported')}>
      <div className="flex items-center gap-2 text-sm font-semibold"><FileArchive className="size-4" /> {plan.zip_name}</div>
      {plan.resources.length > 0 && (
        <ul className="divide-y divide-border rounded-md border border-border">
          {plan.resources.map((r) => (
            <li key={r.zip_path || r.name} className="flex flex-wrap items-center gap-2 p-3 text-sm">
              <span className="font-mono text-xs">{r.name}</span>
              {!r.has_fxmanifest ? <Badge tone="plain">{t('resources.paidAssetsTab.resourceSkipped')}</Badge> : (
                <>
                  {r.version && <Badge tone="plain">{t('resources.paidAssetsTab.versionVersion', { version: r.version })}</Badge>}
                  {r.escrowed ? <Badge tone="info">{t('resources.paidAssetsTab.protectedEscrowed')}</Badge> : <Badge tone="plain">{t('resources.paidAssetsTab.notProtected')}</Badge>}
                  {r.escrowed && (r.has_fxap ? <Badge tone="success">{t('resources.paidAssetsTab.fxapFilePresent')}</Badge> : <Badge tone="danger">{t('resources.paidAssetsTab.fxapFileMissing')}</Badge>)}
                  {r.replaces && <Badge tone="amber">{t('resources.paidAssetsTab.replacesImportedCopyKept')}</Badge>}
                  <span className="ml-auto font-mono text-xs text-muted-foreground">{t('resources.paidAssetsTab.resourcesPaidNameFilesize', { name: r.name, fileSize: fileSize(r.size) })}</span>
                </>
              )}
            </li>
          ))}
        </ul>
      )}
      {blocking.length > 0 && (
        <Alert tone="danger" title={t('resources.paidAssetsTab.zipCantImported')}>
          <ul className="list-disc pl-5">{blocking.map((p, i) => <li key={i}>{p.message}</li>)}</ul>
        </Alert>
      )}
      {warnings.length > 0 && (
        <Alert tone="warning" title={t('resources.paidAssetsTab.checkBeforeImporting')}>
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
      setErr(new Error(t('resources.paidAssetsTab.chooseZipFileDownloaded')))
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
        aria-label={t('resources.paidAssetsTab.dropZipFileHere')}
        className={`flex flex-col items-center gap-3 rounded-md border-2 border-dashed p-6 text-center text-sm ${dragging ? 'border-primary bg-primary/5' : 'border-border'}`}
        onDragOver={(e) => { e.preventDefault(); if (canImport) setDragging(true) }}
        onDragLeave={() => setDragging(false)}
        onDrop={(e) => { e.preventDefault(); setDragging(false); if (canImport) pick(e.dataTransfer.files) }}
      >
        <Upload className="size-6 text-muted-foreground" />
        <p>{t('resources.paidAssetsTab.dropAssetsZipFile')}</p>
        <input ref={fileInput} id="asset-zip" type="file" accept=".zip,application/zip" className="sr-only" disabled={!canImport} onChange={(e) => { pick(e.target.files); e.target.value = '' }} />
        <Button asChild variant="secondary" size="sm"><label htmlFor="asset-zip" aria-disabled={!canImport} className={canImport ? 'cursor-pointer' : 'pointer-events-none opacity-50'}>{t('resources.paidAssetsTab.chooseZipFile')}</label></Button>
      </div>
      {local && canImport && downloads.data && (
        <div className="space-y-2">
          <div className="text-sm font-semibold">{t('resources.paidAssetsTab.recentZipsDownloadsFolder')}</div>
          {downloads.data.zips.length === 0 ? (
            <p className="text-sm text-muted-foreground">{downloads.data.found ? t('resources.paidAssetsTab.noZipFilesWere') : t('resources.paidAssetsTab.noDownloadsFolderFound')}</p>
          ) : (
            <ul className="divide-y divide-border rounded-md border border-border">
              {downloads.data.zips.map((z) => (
                <li key={z.name} className="flex flex-wrap items-center gap-2 p-2 text-sm">
                  <span className="font-mono text-xs">{z.name}</span>
                  <span className="text-xs text-muted-foreground">{fileSize(z.size)} · {timeAgo(z.modified_unix * 1000, Date.now())}</span>
                  <Button size="sm" className="ml-auto" disabled={inspect.isPending} onClick={() => inspect.mutate({ download: z.name })}>{t('resources.paidAssetsTab.checkZip')}</Button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
      {inspect.isPending && <p className="text-sm text-muted-foreground">{t('resources.paidAssetsTab.readingZip')}</p>}
      <ErrorNotice error={err ?? inspect.error ?? (run.phase === 'failed' ? run.error : null)} />
      {inspected && <PlanView plan={inspected.plan} />}
      {inspected?.plan.can_import && source && run.phase !== 'done' && (
        <div className="flex flex-wrap gap-2">
          <Button variant="default" disabled={run.phase === 'running'} onClick={() => void startImport()}>{t('resources.paidAssetsTab.importCountAssetValue', { count })}</Button>
          <Button disabled={run.phase === 'running'} onClick={() => setInspected(null)}>{t('resources.paidAssetsTab.cancel')}</Button>
        </div>
      )}
      {run.phase === 'running' && <p className="text-sm text-muted-foreground">{status.at(-1) ?? t('resources.paidAssetsTab.importing')}</p>}
      {run.phase === 'done' && run.result && (
        <Alert tone="success" title={t('resources.paidAssetsTab.imported')}>
          <ul className="list-disc pl-5">
            {run.result.imported.map((a) => <li key={a.name}><span className="font-mono">{a.name}</span>{a.version ? ` ${a.version}` : ''}{a.replaced ? t('resources.paidAssetsTab.replacedOldCopyKept') : ''}</li>)}
          </ul>
          {run.result.ensured.length > 0 && <p>{t('resources.paidAssetsTab.addedServerCfgJoin', { join: run.result.ensured.map((e) => `ensure ${e}`).join(', ') })}</p>}
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
        {a.version && <Badge tone="plain">{t('resources.paidAssetsTab.versionVersion', { version: a.version })}</Badge>}
        {a.imported_unix && <span className="text-xs text-muted-foreground">{t('resources.paidAssetsTab.importedTolocaledatestring', { when: unix(a.imported_unix) })}</span>}
        {!a.present && <Badge tone="danger">{t('resources.paidAssetsTab.folderMissing')}</Badge>}
        {a.present && !a.has_fxap && <Badge tone="amber">{t('resources.paidAssetsTab.noFxapFile')}</Badge>}
        {canChange && (
          <span className="ml-auto flex gap-2">
            <Button size="sm" onClick={onReplace}>{t('resources.paidAssetsTab.replaceNewerZip')}</Button>
            <Button size="sm" variant="outline" onClick={() => setConfirm(true)}>{t('resources.paidAssetsTab.remove')}</Button>
          </span>
        )}
      </div>
      <ErrorNotice error={m.error} />
      <ConfirmDialog
        open={confirm}
        onOpenChange={setConfirm}
        title={t('resources.paidAssetsTab.removeName', { name: a.name })}
        description={t('resources.paidAssetsTab.itsFolderMovedBackups')}
        confirmLabel={t('resources.paidAssetsTab.remove')}
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
          <CardHeader><CardTitle>{t('resources.paidAssetsTab.paidAssets')}</CardTitle></CardHeader>
          <CardContent className="space-y-3 text-sm">
            <p>{t('resources.paidAssetsTab.assetsBuyCfxRe')}</p>
            <Alert tone="info">{t('resources.paidAssetsTab.buyDownloadSameCfx')}</Alert>
            <Button asChild variant="default">
              <a href={portal} target="_blank" rel="noopener noreferrer">{rich('resources.paidAssetsTab.openCfxRePortal', { externalLink: <ExternalLink /> })}</a>
            </Button>
          </CardContent>
        </Card>
        <Hints hints={list.data?.hints ?? []} />
        <Card>
          <CardHeader><CardTitle>{t('resources.paidAssetsTab.importedPaidAssetsValue', { value: noList ? '' : ` (${assets.length})` })}</CardTitle></CardHeader>
          <CardContent className="space-y-3">
            <ErrorNotice error={list.error} />
            {removed && <Alert tone="success" title={t('resources.paidAssetsTab.removed')}>{removed.map((n, i) => <p key={i}>{n}</p>)}</Alert>}
            {noList ? <Alert tone="info">{noList}</Alert> : assets.length === 0 && !list.isLoading ? <p className="text-sm text-muted-foreground">{t('resources.paidAssetsTab.noPaidAssetsBeen')}</p> : (
              <ul className="divide-y divide-border rounded-md border border-border">
                {assets.map((a) => <AssetRow key={a.name} server={s.name} asset={a} canChange={canImport} onReplace={() => fileInput.current?.click()} onRemoved={(n) => { setRemoved(n); refresh() }} />)}
              </ul>
            )}
          </CardContent>
        </Card>
      </div>
      <Card>
        <CardHeader><CardTitle>{t('resources.paidAssetsTab.importPaidAsset')}</CardTitle></CardHeader>
        <CardContent className="space-y-3">
          {!canImport && <Alert tone="info">{whyNot(role, 'admin')}</Alert>}
          <ImportPanel server={s} canImport={canImport} local={local} onDone={refresh} fileInput={fileInput} />
        </CardContent>
      </Card>
    </div>
  )
}
