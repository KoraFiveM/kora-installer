import { cn } from '@/lib/utils'
import { t } from '@/i18n'
import '@/i18n/en/core'

/**
 * A live region for screen readers that is not drawn. Put it next to whatever changes (a progress bar, a list of steps)
 * and give it a message that changes only at moments worth hearing: a start, a milestone, the end. Do not put
 * `aria-live` on the changing thing itself: a bar that moves every second would be read out every second.
 *  - polite (default): said when the reader is idle. Starts, milestones, completion.
 *  - `assertive`: said at once, for a failure that needs attention.
 * The region exists from the first render (an empty one is created first and filled later): a region added together
 * with its text is often missed by screen readers.
 */
export function Announcer({ message, assertive = false, className }: { message: string; assertive?: boolean; className?: string }) {
  return (
    <div role={assertive ? 'alert' : 'status'} aria-live={assertive ? 'assertive' : 'polite'} aria-atomic="true" className={cn('sr-only', className)}>
      {message}
    </div>
  )
}

/** The milestone a percentage has reached for announcing: nothing before it starts, then every quarter, then "finished". */
export function milestoneOf(percent: number, done = false): 'start' | 25 | 50 | 75 | 'done' | null {
  if (done || percent >= 100) return 'done'
  if (percent >= 75) return 75
  if (percent >= 50) return 50
  if (percent >= 25) return 25
  return percent > 0 ? 'start' : null
}

/** "Backup progress: started / 25 per cent / ... / finished" at the start, each quarter and the end; the same words in between, so nothing new is said. */
export function progressSentence(label: string, percent: number, done = false): string {
  const m = milestoneOf(percent, done)
  return m === null ? '' : m === 'done' ? `${label}: finished` : m === 'start' ? `${label}: started` : t('core.announcer.labelMPerCent', { label, m })
}
