import * as React from 'react'
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from '@/lib/utils'

const badgeVariants = cva('inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs font-medium', {
  variants: {
    tone: {
      neutral: 'border-steel/40 text-steel bg-steel/10',
      success: 'border-success/40 text-success bg-success/10',
      amber: 'border-amber/40 text-amber bg-amber/10',
      danger: 'border-destructive/40 text-destructive bg-destructive/10',
      info: 'border-info/40 text-info bg-info/10',
      orange: 'border-orange/40 text-orange bg-orange/10',
      plain: 'border-border text-muted-foreground',
    },
  },
  defaultVariants: { tone: 'neutral' },
})
export type Tone = NonNullable<VariantProps<typeof badgeVariants>['tone']>
export interface BadgeProps extends React.HTMLAttributes<HTMLSpanElement>, VariantProps<typeof badgeVariants> {
  dot?: boolean
}
export const Badge = ({ className, tone, dot, children, ...p }: BadgeProps) => (
  <span className={cn(badgeVariants({ tone }), className)} {...p}>
    {dot && <span aria-hidden className="size-1.5 rounded-full bg-current" />}
    {children}
  </span>
)
