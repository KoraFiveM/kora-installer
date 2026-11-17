import { useState } from 'react'
import { Dialog, DialogContent } from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Alert } from '@/components/ui/alert'
import type { ChangesChoice } from '@/api/types'
import { t } from '@/i18n'
import '@/i18n/en/resources/changesDialog'

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
        title={sure ? t('resources.changesDialog.throwChangesAway') : t('resources.changesDialog.changesSaved')}
        description={many ? t('resources.changesDialog.countResourcesChangesOwn', { count: resources.length, join: resources.join(', ') }) : t('resources.changesDialog.valueChangesOwnWere', { value: resources[0] ?? t('resources.changesDialog.thisResource') })}
      >
        {files.length > 0 && (
          <div className="mb-3 text-xs text-muted-foreground">
            <div className="mb-1 font-semibold">{t('resources.changesDialog.changedFiles')}</div>
            <ul className="max-h-28 list-disc overflow-y-auto pl-5 font-mono">{files.map((f) => <li key={f}>{f}</li>)}</ul>
          </div>
        )}
        {sure ? (
          <>
            <Alert tone="danger" title={t('resources.changesDialog.cannotUndone')}>{t('resources.changesDialog.changedFilesGoBack', { value: many ? 'those folders' : 'that folder' })}</Alert>
            <div className="mt-5 flex flex-wrap justify-end gap-2">
              <Button onClick={() => setSure(false)} disabled={busy}>{t('resources.changesDialog.goBack')}</Button>
              <Button variant="destructive" disabled={busy} onClick={() => { setSure(false); onChoose('discard') }}>{t('resources.changesDialog.throwMyChangesAway')}</Button>
            </div>
          </>
        ) : (
          <>
            <p className="text-sm text-muted-foreground">{t('resources.changesDialog.koraSwitchUntilChoose')}</p>
            <div className="mt-4 space-y-3">
              <div>
                <Button variant="default" disabled={busy} onClick={() => onChoose('stash')}>{t('resources.changesDialog.keepMyChangesAside')}</Button>
                <p className="mt-1 text-xs text-muted-foreground">{t('resources.changesDialog.recommendedTheySetAside')}</p>
              </div>
              <div>
                <Button variant="destructive" disabled={busy} onClick={() => setSure(true)}>{t('resources.changesDialog.throwMyChangesAway2')}</Button>
                <p className="mt-1 text-xs text-muted-foreground">{t('resources.changesDialog.askedConfirm')}</p>
              </div>
              <div><Button disabled={busy} onClick={close}>{t('resources.changesDialog.cancelChangeNothing')}</Button></div>
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}
