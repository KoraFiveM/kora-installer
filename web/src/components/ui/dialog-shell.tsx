import type { ReactNode } from 'react'
import { Dialog, DialogContent } from '@/components/ui/dialog'

/** The dialog itself, in its own file so the dialog library (Radix) is fetched when the first dialog opens, not with the first screen. */
export default function DialogShell({ onOpenChange, title, description, children, className }: { onOpenChange: (o: boolean) => void; title: string; description?: string; children: ReactNode; className?: string }) {
  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent title={title} description={description} className={className}>{children}</DialogContent>
    </Dialog>
  )
}
