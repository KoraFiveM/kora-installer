/**
 * The fix buttons that work anywhere a Problem carries them (install wizard, GitHub tab, System check):
 *
 * - `install_tool` ("Install it for me"): downloads Kora's own copy of Git or the GitHub CLI, with a progress bar;
 * - `sign_in_github` ("Sign in to GitHub" / "Sign in again"): the panel's one-time-code sign-in;
 * - `start_database` ("Start it for me"): starts the installed database server.
 *
 * Each opens a small dialog that does the work, shows progress, and says plainly what happened. Wrap the app
 * in `<ToolchainActions>` once; it registers the handlers with `<ProblemActions>` (see lib/problemActions.tsx).
 */
import { Suspense, lazy, useMemo, useRef, useState, type ReactNode } from 'react'
import { useRole } from '@/hooks/useRole'
import { ProblemActions, type ActionHandlers } from '@/lib/problemActions'
import type { Request } from '@/components/ToolchainDialog'

// The dialog's contents (progress, GitHub sign-in) are fetched when a fix is first started.
const ToolchainDialog = lazy(() => import('@/components/ToolchainDialog'))

export function ToolchainActions({ children }: { children: ReactNode }) {
  const [req, setReq] = useState<Request | null>(null)
  const seq = useRef(0)
  // Installing tools, signing in to GitHub and starting the database need an administrator: for anyone else the
  // buttons are not drawn (the System check says who can fix it) instead of ending in a refusal.
  const isAdmin = useRole().can('admin')
  const handlers = useMemo<ActionHandlers>(
    () => !isAdmin ? {} : ({
      install_tool: (a, c) => {
        if (a.tool === 'git' || a.tool === 'gh') setReq({ kind: 'install', tool: a.tool, id: ++seq.current, afterFix: c.afterFix })
      },
      sign_in_github: (_, c) => setReq({ kind: 'signin', id: ++seq.current, afterFix: c.afterFix }),
      start_database: (_, c) => setReq({ kind: 'database', id: ++seq.current, afterFix: c.afterFix }),
    }),
    [isAdmin],
  )
  return (
    <ProblemActions handlers={handlers}>
      {children}
      {req && (
        <Suspense fallback={null}>
          <ToolchainDialog req={req} onClose={() => setReq(null)} />
        </Suspense>
      )}
    </ProblemActions>
  )
}
