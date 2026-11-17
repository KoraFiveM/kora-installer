import { useState } from 'react'
import { Dialog, DialogContent } from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Alert } from '@/components/ui/alert'
import type { ChangesChoice } from '@/api/types'

/**
 * "Your changes are in the way": asked when a resource has changes of the person's own that were never saved anywhere.
 * Three plain choices: keep them aside (the safe one), throw them away (asked again, with a warning), or cancel.
 */
export function ChangesDialog({ open, resources, files, busy, onChoose, onCancel }: {
  open: boolean
  /** Which resources have changes (one name for a single switch). */
  resources: string[]
  /** The files that changed, as far as they are known. */
  files: string[]
  busy?: boolean
  onChoose: (c: Exclude<ChangesChoice, 'ask'>) => void
  onCancel: () => void
}) {
  const [sure, setSure] = useState(false)
  const many = resources.length > 1
  const close = () => { setSure(false); onCancel() }
  return (
    <Dialog open={open} onOpenChange={(o) => { if (!o) close() }}>
      <DialogContent
        title={sure ? 'Throw your changes away?' : 'You have changes that are not saved'}
        description={many ? `${resources.length} resources have changes of your own: ${resources.join(', ')}.` : `${resources[0] ?? 'This resource'} has changes of your own that were never saved as a commit.`}
      >
        {files.length > 0 && (
          <div className="mb-3 text-xs text-muted-foreground">
            <div className="mb-1 font-semibold">Changed files</div>
            <ul className="max-h-28 list-disc overflow-y-auto pl-5 font-mono">{files.map((f) => <li key={f}>{f}</li>)}</ul>
          </div>
        )}
        {sure ? (
          <>
            <Alert tone="danger" title="This cannot be undone">The changed files go back to how they were, and any new files you added in {many ? 'those folders' : 'that folder'} are deleted. Kora keeps no copy.</Alert>
            <div className="mt-5 flex flex-wrap justify-end gap-2">
              <Button onClick={() => setSure(false)} disabled={busy}>Go back</Button>
              <Button variant="destructive" disabled={busy} onClick={() => { setSure(false); onChoose('discard') }}>Throw my changes away</Button>
            </div>
          </>
        ) : (
          <>
            <p className="text-sm text-muted-foreground">Kora will not switch until you choose, so nothing is lost by accident.</p>
            <div className="mt-4 space-y-3">
              <div>
                <Button variant="default" disabled={busy} onClick={() => onChoose('stash')}>Keep my changes aside</Button>
                <p className="mt-1 text-xs text-muted-foreground">Recommended. They are set aside safely and you can bring them back later from this panel.</p>
              </div>
              <div>
                <Button variant="destructive" disabled={busy} onClick={() => setSure(true)}>Throw my changes away…</Button>
                <p className="mt-1 text-xs text-muted-foreground">You will be asked to confirm.</p>
              </div>
              <div><Button disabled={busy} onClick={close}>Cancel, change nothing</Button></div>
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}
