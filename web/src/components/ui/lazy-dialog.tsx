import { Suspense, lazy, type ReactNode } from 'react'

const Shell = lazy(() => import('./dialog-shell'))

/**
 * A dialog for components that are part of the first screen: nothing is loaded until it opens (the dialog library is about
 * 18 kB gzipped, and the servers list does not need it). Components that are only reached through a lazy page use
 * `Dialog`/`DialogContent` from ui/dialog directly.
 */
export function LazyDialog({ open, ...p }: { open: boolean; onOpenChange: (o: boolean) => void; title: string; description?: string; children: ReactNode; className?: string }) {
  return open ? (
    <Suspense fallback={null}>
      <Shell {...p} />
    </Suspense>
  ) : null
}
