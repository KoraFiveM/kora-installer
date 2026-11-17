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
      <DialogContent title="Create a support bundle" description="One file you can hand to whoever is helping you, with your secrets removed.">
        <div className="space-y-4">
          {!canUse && <Alert tone="info">Your account ({role.role}) cannot make a support bundle. Ask an administrator.</Alert>}
          <Alert tone="info">Kora never uploads this file anywhere. It is saved on this computer, and it leaves only if you send it yourself. Licence keys, database logins, txAdmin accounts and tokens, webhook addresses, Git tokens and passwords are removed first.</Alert>
          {!listing && (
            <div className="space-y-2">
              <p className="text-sm text-muted-foreground">Kora will collect its version, your system, the System check, tool versions, your servers' settings (without secrets), recent jobs and problems, the activity log, install and update logs, the service and firewall state, and disk space. You see everything before it is saved.</p>
              <Button variant="default" disabled={!canUse || prepare.isPending} onClick={() => prepare.mutate()}><LifeBuoy /> {prepare.isPending ? 'Preparing…' : 'Prepare it and show me what is in it'}</Button>
            </div>
          )}
          {listing && !save.data && (
            <div className="space-y-3">
              <p className="text-sm font-semibold">This is what the file will hold ({size(listing.total_bytes)}). Click a file to read it exactly as it will be saved.</p>
              <ul className="max-h-60 divide-y divide-border overflow-auto rounded-md border border-border" aria-label="Files in the support bundle">
                {listing.files.map((f) => (
                  <li key={f.name} className="flex flex-wrap items-center gap-2 p-2 text-sm">
                    <button type="button" className="font-mono text-xs text-info underline" onClick={() => read.mutate(f.name)}>{f.name}</button>
                    <span className="text-xs text-muted-foreground">{size(f.bytes)}</span>
                    {f.redactions > 0 && <Badge tone="amber">{f.redactions} hidden: {f.hidden.join(', ')}</Badge>}
                  </li>
                ))}
              </ul>
              {listing.omitted.length > 0 && (
                <Alert tone="info" title="Left out altogether, because they hold secrets">
                  <ul className="list-disc pl-5">{listing.omitted.map((o) => <li key={o.name}><span className="font-mono text-xs">{o.name}</span>: it {o.why}</li>)}</ul>
                </Alert>
              )}
              {open && <div><div className="mb-1 text-xs font-semibold">{open}</div><pre aria-label={`Text of ${open}`} className="max-h-56 overflow-auto whitespace-pre-wrap rounded-md border border-border bg-ink p-2 font-mono text-xs">{text || '(empty)'}</pre></div>}
              <p className="text-sm text-muted-foreground">{listing.never_uploaded} It will be saved in <span className="font-mono text-xs">{listing.folder}</span>.</p>
              <div className="flex flex-wrap gap-2">
                <Button variant="default" disabled={save.isPending} onClick={() => save.mutate()}>Save this bundle</Button>
                <Button onClick={onClose}>Cancel, save nothing</Button>
              </div>
            </div>
          )}
          {save.data && (
            <div className="space-y-3">
              <Alert tone="success" title="Saved">Send this one file to whoever is helping you.</Alert>
              <CopyField value={save.data.path} label="Where it was saved" />
              {role.remote && <Button onClick={() => dl.mutate(save.data!.file_name)} disabled={dl.isPending}><Download /> Download it</Button>}
              <div className="flex justify-end"><Button onClick={onClose}>Close</Button></div>
            </div>
          )}
          <ErrorNotice error={prepare.error ?? read.error ?? save.error ?? dl.error} />
        </div>
      </DialogContent>
    </Dialog>
  )
}
