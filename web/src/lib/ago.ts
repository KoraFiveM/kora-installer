import { fmt, t } from '@/i18n'

/** "just now", "2 minutes ago", "3 hours ago", for a moment `then` (ms) seen at `now` (ms): the wording is Intl's, in the current language. */
export function timeAgo(then: number, now: number): string {
  if (Math.abs(now - then) < 45_000) return t('core.ago.justNow')
  return fmt.relative(then, now)
}
