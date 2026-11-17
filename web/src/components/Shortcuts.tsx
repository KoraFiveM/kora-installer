import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import { useRole } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Checkbox } from '@/components/ui/checkbox'
import { ErrorNotice } from '@/components/ErrorNotice'

/** Status of the Kora shortcuts, and the one-click "Tidy up old shortcuts" (see `PUT /app`). */
function useShortcuts() {
  const qc = useQueryClient()
  const q = useQuery({ queryKey: ['app'], queryFn: api.app.status })
  const [desktop, setDesktop] = useState(true)
  const tidy = useMutation({
    mutationFn: () => api.app.install({ desktop_shortcut: desktop, dry_run: false }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['app'] }),
    // The per-user shortcuts are made even when the old copy still needs administrator rights.
    onError: () => qc.invalidateQueries({ queryKey: ['app'] }),
  })
  return { q, desktop, setDesktop, tidy }
}

/** A short notice on the start page, only while old shortcuts are around. */
export function TidyUpBanner() {
  const { q, tidy } = useShortcuts()
  const role = useRole()
  if (!q.data?.legacy || !role.can('admin') || tidy.isSuccess) return null
  return (
    <Alert tone="info">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <span>Kora now opens from your own shortcuts, with no administrator prompt. Tidy up the old ones?</span>
        <Button onClick={() => tidy.mutate()} disabled={tidy.isPending}>Tidy up old shortcuts</Button>
      </div>
      <ErrorNotice error={tidy.error} />
    </Alert>
  )
}

export function ShortcutsCard() {
  const { q, desktop, setDesktop, tidy } = useShortcuts()
  const role = useRole()
  if (q.data && !q.data.supported) return null
  if (!role.can('admin')) return null
  const legacy = q.data?.legacy ?? false
  return (
    <Card>
      <CardHeader><CardTitle>Kora shortcuts</CardTitle></CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">Kora is this panel in your browser. The shortcut in your Start Menu (or applications menu) opens it; no administrator prompt.</p>
        <div className="text-sm">{q.data?.shortcuts ? <Badge tone="success" dot>Shortcut in place</Badge> : <Badge tone="neutral" dot>No shortcut yet</Badge>}</div>
        {legacy && (
          <Alert tone="info">
            <p className="font-medium">An older Kora is still installed on this computer.</p>
            <ul className="mt-1 list-disc pl-5 text-sm">{q.data?.legacy_items.map((i) => <li key={i}>{i}</li>)}</ul>
            <p className="mt-1 text-sm">Tidying up makes the new shortcuts first, so the old ones keep working until the end. Removing the old copy needs administrator rights; if you are not an administrator here, Kora shows the command to run.</p>
          </Alert>
        )}
        {tidy.isSuccess && <Alert tone="success">{tidy.data.message}</Alert>}
        <Checkbox label="Also put a shortcut on the desktop (Windows)" checked={desktop} onChange={(e) => setDesktop(e.target.checked)} />
        <Button onClick={() => tidy.mutate()} disabled={tidy.isPending}>{legacy ? 'Tidy up old shortcuts' : q.data?.shortcuts ? 'Refresh the shortcuts' : 'Add the shortcuts'}</Button>
        <ErrorNotice error={tidy.error ?? q.error} />
      </CardContent>
    </Card>
  )
}
