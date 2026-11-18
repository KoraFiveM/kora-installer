import { useId } from 'react'
import { DEV_LOCALES, offered, setLocale, t } from '@/i18n'
import { useLocale } from '@/i18n/react'
import { Select, Label } from '@/components/ui/input'
import '@/i18n/en/core/language'

/**
 * The language of the panel. The choice is remembered by this browser (`setLocale`); the page is redrawn in the new language
 * at once. Languages marked "beta" are machine-assisted and not yet reviewed by a native speaker. `compact`: no visible label
 * (the sign-in screen), the label is still there for screen readers.
 */
export function LanguagePicker({ compact = false }: { compact?: boolean }) {
  const id = useId()
  const current = useLocale()
  const list = offered(DEV_LOCALES)
  return (
    <div className={compact ? 'flex items-center gap-2' : 'space-y-1.5'}>
      <Label htmlFor={id} className={compact ? 'sr-only' : undefined}>{t('core.language.label')}</Label>
      <Select id={id} value={current} onChange={(e) => void setLocale(e.target.value)} className={compact ? 'w-auto' : 'max-w-xs'} lang={current}>
        {list.map((l) => (
          // The name is in its own language (so someone who cannot read this one can still find theirs).
          <option key={l.code} value={l.code} lang={l.code === 'en-XA' ? 'en' : l.code}>{l.beta ? t('core.language.beta', { name: l.name }) : l.name}</option>
        ))}
      </Select>
    </div>
  )
}
