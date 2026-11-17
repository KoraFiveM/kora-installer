import * as React from 'react'
import * as DialogPrimitive from '@radix-ui/react-dialog'
import { X } from 'lucide-react'
import { cn } from '@/lib/utils'
import { t } from '@/i18n'
import '@/i18n/en/core'

export const Dialog = DialogPrimitive.Root
export const DialogTrigger = DialogPrimitive.Trigger
export const DialogClose = DialogPrimitive.Close

export function DialogContent({ className, children, title, description, onOpenAutoFocus, onCloseAutoFocus, ...p }: Omit<React.ComponentProps<typeof DialogPrimitive.Content>, 'title'> & { title: string; description?: string }) {
  // Dialogs here are opened by state, not by a DialogTrigger, so Radix does not know which control to give focus back to
  // (focus would be lost to the page). Remember what had focus when the dialog opened and return to it when it closes.
  const opener = React.useRef<HTMLElement | null>(null)
  return (
    <DialogPrimitive.Portal>
      <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/70" />
      <DialogPrimitive.Content
        className={cn('fixed left-1/2 top-1/2 z-50 max-h-[90vh] w-[calc(100%-2rem)] max-w-lg -translate-x-1/2 -translate-y-1/2 overflow-y-auto rounded-lg border border-border bg-card p-6 shadow-xl', className)}
        onOpenAutoFocus={(e) => {
          const a = document.activeElement
          opener.current = a instanceof HTMLElement && a !== document.body ? a : null
          onOpenAutoFocus?.(e)
        }}
        onCloseAutoFocus={(e) => {
          onCloseAutoFocus?.(e)
          const el = opener.current
          opener.current = null
          if (!e.defaultPrevented && el?.isConnected) {
            e.preventDefault()
            el.focus()
          }
        }}
        {...p}
      >
        <DialogPrimitive.Title className="font-heading text-lg font-semibold">{title}</DialogPrimitive.Title>
        {description ? <DialogPrimitive.Description className="mt-1 text-sm text-muted-foreground">{description}</DialogPrimitive.Description> : <DialogPrimitive.Description className="sr-only">{title}</DialogPrimitive.Description>}
        <div className="mt-4">{children}</div>
        <DialogPrimitive.Close className="absolute right-3 top-3 rounded-md p-1 text-muted-foreground hover:text-foreground" aria-label={t('core.dialog.close')}>
          <X className="size-4" />
        </DialogPrimitive.Close>
      </DialogPrimitive.Content>
    </DialogPrimitive.Portal>
  )
}
