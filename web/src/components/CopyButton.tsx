import { useState } from 'react'
import { Check, Copy } from 'lucide-react'
import { Button } from '@/components/ui/button'

/** Copies text to the clipboard. Falls back to a hidden textarea where the clipboard API is unavailable (plain http). */
export async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text)
      return true
    }
  } catch {
    /* fall through */
  }
  try {
    const ta = document.createElement('textarea')
    ta.value = text
    ta.style.position = 'fixed'
    ta.style.opacity = '0'
    document.body.appendChild(ta)
    ta.select()
    const ok = document.execCommand('copy')
    ta.remove()
    return ok
  } catch {
    return false
  }
}

export function CopyButton({ text, label = 'Copy', size = 'sm' }: { text: string; label?: string; size?: 'sm' | 'default' }) {
  const [state, setState] = useState<'idle' | 'copied' | 'failed'>('idle')
  return (
    <Button
      size={size}
      variant="secondary"
      onClick={async () => {
        setState((await copyText(text)) ? 'copied' : 'failed')
        setTimeout(() => setState('idle'), 2000)
      }}
    >
      {state === 'copied' ? <Check /> : <Copy />}
      <span aria-live="polite">{state === 'copied' ? 'Copied' : state === 'failed' ? 'Press Ctrl+C' : label}</span>
    </Button>
  )
}

export function CopyField({ value, label }: { value: string; label: string }) {
  return (
    <div className="flex items-center gap-2">
      <code aria-label={label} className="min-w-0 flex-1 truncate rounded-md border border-border bg-ink px-3 py-1.5 font-mono text-xs">{value}</code>
      <CopyButton text={value} />
    </div>
  )
}
