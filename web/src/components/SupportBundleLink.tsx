import { Suspense, lazy, useContext, useState } from 'react'
import { QueryClientContext } from '@tanstack/react-query'
import { useRole } from '@/hooks/useRole'

// The dialog (and what it needs) is fetched the first time someone opens it, so the link costs the first page almost nothing.
const SupportBundleDialog = lazy(() => import('./SupportBundleDialog'))

function Inner({ server, label }: { server?: string; label: string }) {
  const [open, setOpen] = useState(false)
  const role = useRole()
  if (!role.can('admin')) return null
  return (
    <>
      <button type="button" className="text-xs text-info underline" onClick={() => setOpen(true)}>{label}</button>
      {open && (
        <Suspense fallback={null}>
          <SupportBundleDialog server={server} onClose={() => setOpen(false)} />
        </Suspense>
      )}
    </>
  )
}

/** The small link in an error's details area, and wherever else a bundle is offered. Administrators only. */
export function SupportBundleLink({ server, label = 'Create a support bundle' }: { server?: string; label?: string }) {
  // An error can be drawn outside the app's data layer (a crash screen): then there is no role to ask, so no link.
  if (!useContext(QueryClientContext)) return null
  return <Inner server={server} label={label} />
}
