import { Skeleton } from '@/components/Skeleton'

/** Shown while the file of a page or tab is being fetched (a few milliseconds on this computer, longer from afar). */
export function RouteLoading({ label = 'Loading…', className = 'h-64' }: { label?: string; className?: string }) {
  return (
    <div role="status" aria-live="polite">
      <span className="sr-only">{label}</span>
      <Skeleton className={className} />
    </div>
  )
}
