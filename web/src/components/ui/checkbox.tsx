import * as React from 'react'
import { cn } from '@/lib/utils'

/** A native checkbox with a visible label (keyboard and screen-reader friendly). */
export function Checkbox({ label, hint, className, ...p }: Omit<React.InputHTMLAttributes<HTMLInputElement>, 'type'> & { label: React.ReactNode; hint?: React.ReactNode }) {
  return (
    <label className={cn('flex cursor-pointer items-start gap-3 text-sm has-[:disabled]:cursor-not-allowed has-[:disabled]:opacity-60', className)}>
      <input type="checkbox" className="mt-0.5 size-4 shrink-0 accent-lime" {...p} />
      <span>
        <span className="font-medium">{label}</span>
        {hint && <span className="mt-0.5 block text-muted-foreground">{hint}</span>}
      </span>
    </label>
  )
}

export function Radio({ label, hint, className, ...p }: Omit<React.InputHTMLAttributes<HTMLInputElement>, 'type'> & { label: React.ReactNode; hint?: React.ReactNode }) {
  return (
    <label className={cn('flex cursor-pointer items-start gap-3 rounded-md border border-border p-3 text-sm has-[:checked]:border-primary/70 has-[:disabled]:opacity-60', className)}>
      <input type="radio" className="mt-0.5 size-4 shrink-0 accent-lime" {...p} />
      <span>
        <span className="font-medium">{label}</span>
        {hint && <span className="mt-0.5 block text-muted-foreground">{hint}</span>}
      </span>
    </label>
  )
}
