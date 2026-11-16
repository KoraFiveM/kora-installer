import * as React from 'react'
import { cva, type VariantProps } from 'class-variance-authority'
import { AlertTriangle, CheckCircle2, Info, XCircle } from 'lucide-react'
import { cn } from '@/lib/utils'

const alertVariants = cva('flex gap-3 rounded-lg border p-4 text-sm', {
  variants: {
    tone: {
      info: 'border-info/40 bg-info/10 [&>svg]:text-info',
      success: 'border-success/40 bg-success/10 [&>svg]:text-success',
      warning: 'border-amber/40 bg-amber/10 [&>svg]:text-amber',
      danger: 'border-destructive/50 bg-destructive/10 [&>svg]:text-destructive',
    },
  },
  defaultVariants: { tone: 'info' },
})
const icons = { info: Info, success: CheckCircle2, warning: AlertTriangle, danger: XCircle }

export interface AlertProps extends Omit<React.HTMLAttributes<HTMLDivElement>, 'title'>, VariantProps<typeof alertVariants> {
  title?: React.ReactNode
}
export function Alert({ className, tone = 'info', title, children, ...p }: AlertProps) {
  const Icon = icons[tone ?? 'info']
  return (
    <div role={tone === 'danger' || tone === 'warning' ? 'alert' : 'status'} className={cn(alertVariants({ tone }), className)} {...p}>
      <Icon className="mt-0.5 size-4 shrink-0" aria-hidden />
      <div className="min-w-0 flex-1 space-y-1">
        {title && <div className="font-semibold">{title}</div>}
        <div className="text-foreground/90">{children}</div>
      </div>
    </div>
  )
}
