import { Suspense, lazy, useEffect, useMemo, useState } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { BrowserRouter, Route, Routes } from 'react-router-dom'
import { ApiError, setUnauthorizedHandler } from '@/api/client'
import { AlertsProvider } from '@/lib/app'
import { signIn, type BootResult } from '@/lib/session'
import { AppShell } from '@/components/AppShell'
import { SignedOut } from '@/components/SignedOut'
import ServersPage from '@/pages/ServersPage'
import ServerDetailPage from '@/pages/ServerDetailPage'
import InstallWizard from '@/pages/InstallWizard'
import FleetPage from '@/pages/FleetPage'
import SettingsPage from '@/pages/SettingsPage'
import NotFound from '@/pages/NotFound'

// The styleguide only exists in mock builds: the constant is false in production, so this branch (and the page) is dropped.
const Styleguide = __KORA_MOCK__ ? lazy(() => import('@/pages/Styleguide')) : null

export function App({ boot }: { boot: BootResult }) {
  const client = useMemo(
    () =>
      new QueryClient({
        defaultOptions: {
          queries: { retry: (n, e) => !(e instanceof ApiError && e.status < 500 && e.status !== 0) && n < 2, refetchOnWindowFocus: false, staleTime: 2000 },
        },
      }),
    [],
  )
  const [signedOut, setSignedOut] = useState<ApiError | boolean>(boot.status === 'failed' ? boot.error : false)

  useEffect(() => {
    setUnauthorizedHandler(() => setSignedOut((s) => s || true))
    return () => setUnauthorizedHandler(null)
  }, [])

  if (signedOut) {
    return (
      <SignedOut
        error={signedOut instanceof ApiError ? signedOut : null}
        onToken={async (t) => {
          const r = await signIn(t)
          if (r.status === 'signed_in') {
            client.clear()
            setSignedOut(false)
            return null
          }
          return r.status === 'failed' ? r.error : null
        }}
      />
    )
  }

  return (
    <QueryClientProvider client={client}>
      <AlertsProvider>
        <BrowserRouter>
          <Routes>
            {Styleguide && <Route path="styleguide" element={<Suspense fallback={null}><Styleguide /></Suspense>} />}
            <Route element={<AppShell />}>
              <Route index element={<ServersPage />} />
              <Route path="servers/:name" element={<ServerDetailPage />} />
              <Route path="install" element={<InstallWizard />} />
              <Route path="fleet" element={<FleetPage />} />
              <Route path="settings" element={<SettingsPage />} />
              <Route path="*" element={<NotFound />} />
            </Route>
          </Routes>
        </BrowserRouter>
      </AlertsProvider>
    </QueryClientProvider>
  )
}
