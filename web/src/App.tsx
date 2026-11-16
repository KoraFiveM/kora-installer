import { useEffect, useMemo, useState } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { BrowserRouter, Route, Routes } from 'react-router-dom'
import { ApiError, setUnauthorizedHandler } from '@/api/client'
import { AlertsProvider } from '@/lib/app'
import { signIn, signInByIdentity, type BootResult } from '@/lib/session'
import { AppShell } from '@/components/AppShell'
import { SignedOut } from '@/components/SignedOut'
import ServersPage from '@/pages/ServersPage'
import ServerDetailPage from '@/pages/ServerDetailPage'
import InstallWizard from '@/pages/InstallWizard'
import SettingsPage from '@/pages/SettingsPage'
import NotFound from '@/pages/NotFound'

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
        onIdentity={async () => {
          const r = await signInByIdentity()
          if (r.status === 'signed_in') {
            client.clear()
            setSignedOut(false)
            return null
          }
          return r.status === 'failed' ? r.error : null
        }}
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
            <Route element={<AppShell />}>
              <Route index element={<ServersPage />} />
              <Route path="servers/:name" element={<ServerDetailPage />} />
              <Route path="install" element={<InstallWizard />} />
              <Route path="settings" element={<SettingsPage />} />
              <Route path="*" element={<NotFound />} />
            </Route>
          </Routes>
        </BrowserRouter>
      </AlertsProvider>
    </QueryClientProvider>
  )
}
