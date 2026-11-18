import * as React from 'react'
import { cn } from '@/lib/utils'

export const Card = ({ className, ...p }: React.HTMLAttributes<HTMLDivElement>) => <div className={cn('rounded-lg border border-border bg-card text-card-foreground', className)} {...p} />
export const CardHeader = ({ className, ...p }: React.HTMLAttributes<HTMLDivElement>) => <div className={cn('flex flex-col gap-1 p-5 pb-3', className)} {...p} />
/** A card's title. An h2 by default (the page's own title is the h1); `as` for the rare card that is the page (sign-in) or sits deeper. */
export const CardTitle = ({ className, as: Tag = 'h2', ...p }: React.HTMLAttributes<HTMLHeadingElement> & { as?: 'h1' | 'h2' | 'h3' }) => <Tag className={cn('text-base font-semibold', className)} {...p} />
export const CardDescription = ({ className, ...p }: React.HTMLAttributes<HTMLParagraphElement>) => <p className={cn('text-sm text-muted-foreground', className)} {...p} />
export const CardContent = ({ className, ...p }: React.HTMLAttributes<HTMLDivElement>) => <div className={cn('p-5 pt-2', className)} {...p} />
