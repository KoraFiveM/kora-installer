import { useState } from 'react'
import { useMutation } from '@tanstack/react-query'
import { Download, LifeBuoy } from 'lucide-react'
import { api } from '@/api/client'
import type { BundleListing } from '@/api/types'
import { useRole } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent } from '@/components/ui/dialog'
import { CopyField } from '@/components/CopyButton'
import { ErrorNotice } from '@/components/ErrorNotice'
import { downloadFile } from '@/lib/download'
import { t } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/errors'

const size = (n: number) => (n < 1024 ? `${n} bytes` : n < 1024 * 1024 ? `${(n / 1024).toFixed(1)} KB` : `${(n / 1024 / 1024).toFixed(1)} MB`)

/**
 * "Create a support bundle": one file to hand to whoever is helping. Kora prepares it, shows exactly what is in it
 * (and any file's text) before anything is saved, and never uploads it. What is saved is the very thing shown.
 */
export default function SupportBundleDialog({ server, onClose }: { server?: string; onClose: () => void }) {
  const role = useRole()
  const [listing, setListing] = useState<BundleListing | null>(null)
  const [open, setOpen] = useState<string | null>(null)
  const [text, setText] = useState<string>('')
  const prepare = useMutation({ mutationFn: () => api.support.preview(server), onSuccess: (l) => setListing(l) })
  const read = useMutation({ mutationFn: (name: string) => api.support.file(listing!.id, name), onSuccess: (f) => { setOpen(f.name); setText(f.text) } })
  const save = useMutation({ mutationFn: () => api.support.create(listing!.id) })
  const dl = useMutation({ mutationFn: (file: string) => downloadFile(api.support.fileUrl(file), file, save.data?.bytes ?? 0) })
  const canUse = role.can('admin')
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent title={t('errors.supportBundleDialog.createSupportBundle')} description={t('errors.supportBundleDialog.oneFileHandWhoever')}>
        <div className="space-y-4">
          {!canUse && <Alert tone="info">{t('errors.supportBundleDialog.accountRoleCannotMake', { role: role.role })}</Alert>}
          <Alert tone="info">{t('errors.supportBundleDialog.koraNeverUploadsFile')}</Alert>
          {!listing && (
            <div className="space-y-2">
              <p className="text-sm text-muted-foreground">{t('errors.supportBundleDialog.koraCollectItsVersion')}</p>
              <Button variant="default" disabled={!canUse || prepare.isPending} onClick={() => prepare.mutate()}><LifeBuoy /> {prepare.isPending ? t('errors.supportBundleDialog.preparing') : t('errors.supportBundleDialog.prepareShowMeWhat')}</Button>
            </div>
          )}
          {listing && !save.data && (
            <div className="space-y-3">
              <p className="text-sm font-semibold">{t('errors.supportBundleDialog.whatFileHoldSize', { size: size(listing.total_bytes) })}</p>
              <ul className="max-h-60 divide-y divide-border overflow-auto rounded-md border border-border" aria-label={t('errors.supportBundleDialog.filesSupportBundle')}>
                {listing.files.map((f) => (
                  <li key={f.name} className="flex flex-wrap items-center gap-2 p-2 text-sm">
                    <button type="button" className="font-mono text-xs text-info underline" onClick={() => read.mutate(f.name)}>{f.name}</button>
                    <span className="text-xs text-muted-foreground">{size(f.bytes)}</span>
                    {f.redactions > 0 && <Badge tone="amber">{t('errors.supportBundleDialog.redactionsHiddenJoin', { redactions: f.redactions, join: f.hidden.join(', ') })}</Badge>}
                  </li>
                ))}
              </ul>
              {listing.omitted.length > 0 && (
                <Alert tone="info" title={t('errors.supportBundleDialog.leftOutAltogetherBecause')}>
                  <ul className="list-disc pl-5">{listing.omitted.map((o) => <li key={o.name}>{rich('errors.supportBundleDialog.spanNodeWhy', { span: <span className="font-mono text-xs">{o.name}</span>, why: o.why })}</li>)}</ul>
                </Alert>
              )}
              {open && <div><div className="mb-1 text-xs font-semibold">{open}</div><pre aria-label={t('errors.supportBundleDialog.textOpen', { open })} className="max-h-56 overflow-auto whitespace-pre-wrap rounded-md border border-border bg-ink p-2 font-mono text-xs">{text || t('errors.supportBundleDialog.empty')}</pre></div>}
              <p className="text-sm text-muted-foreground">{rich('errors.supportBundleDialog.neverUploadedSavedSpan', { never_uploaded: listing.never_uploaded, span: <span className="font-mono text-xs">{listing.folder}</span> })}</p>
              <div className="flex flex-wrap gap-2">
                <Button variant="default" disabled={save.isPending} onClick={() => save.mutate()}>{t('errors.supportBundleDialog.saveBundle')}</Button>
                <Button onClick={onClose}>{t('errors.supportBundleDialog.cancelSaveNothing')}</Button>
              </div>
            </div>
          )}
          {save.data && (
            <div className="space-y-3">
              <Alert tone="success" title={t('errors.supportBundleDialog.saved')}>{t('errors.supportBundleDialog.sendOneFileWhoever')}</Alert>
              <CopyField value={save.data.path} label={t('errors.supportBundleDialog.whereSaved')} />
              {role.remote && <Button onClick={() => dl.mutate(save.data!.file_name)} disabled={dl.isPending}>{rich('errors.supportBundleDialog.downloadNodeDownload', { download: <Download /> })}</Button>}
              <div className="flex justify-end"><Button onClick={onClose}>{t('errors.supportBundleDialog.close')}</Button></div>
            </div>
          )}
          <ErrorNotice error={prepare.error ?? read.error ?? save.error ?? dl.error} />
        </div>
      </DialogContent>
    </Dialog>
  )
}
