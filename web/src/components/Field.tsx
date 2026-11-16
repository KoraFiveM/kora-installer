import { useId, type ReactElement, cloneElement } from 'react'
import { Label } from '@/components/ui/input'

/** Label + control + hint + error, wired together for screen readers. */
export function Field({ label, hint, error, children }: { label: string; hint?: React.ReactNode; error?: string | null; children: ReactElement<Record<string, unknown>> }) {
  const id = useId()
  const describedBy = [hint ? `${id}-hint` : '', error ? `${id}-err` : ''].filter(Boolean).join(' ') || undefined
  return (
    <div className="space-y-1.5">
      <Label htmlFor={id}>{label}</Label>
      {cloneElement(children, { id, 'aria-describedby': describedBy, 'aria-invalid': error ? true : undefined })}
      {hint && <p id={`${id}-hint`} className="text-xs text-muted-foreground">{hint}</p>}
      {error && <p id={`${id}-err`} role="alert" className="text-xs text-destructive">{error}</p>}
    </div>
  )
}
