import { Suspense, lazy, useEffect, useMemo, useState } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { BrowserRouter, Route, Routes } from 'react-router-dom'
import { ApiError, setUnauthorizedHandler } from '@/api/client'
import { AlertsProvider } from '@/lib/app'
import { useLocale } from '@/i18n/react'
import { clearHash, enrolFromHash, signIn, signInByIdentity, type BootResult } from '@/lib/session'
import { AppShell } from '@/components/AppShell'
import { AdminActions } from '@/components/AdminActions'
import { ToolchainActions } from '@/components/ToolchainActions'
import { SignedOut } from '@/components/SignedOut'
import { Enrol } from '@/components/auth/Enrol'
import { ReauthProvider } from '@/components/auth/ReauthProvider'
import ServersPage from '@/pages/ServersPage'
import NotFound from '@/pages/NotFound'

// Everything except the server list (the landing page) is fetched when it is first opened, so the first load only
// carries what the first screen needs. A file that cannot be fetched (the panel was updated meanwhile) ends in the
// ErrorBoundary of the app shell, which offers a reload: see lib/chunkError.ts. The shell shows the loading state.
const ServerDetailPage = lazy(() => import('@/pages/ServerDetailPage'))
const ResourcePage = lazy(() => import('@/pages/ResourcePage'))
const InstallWizard = lazy(() => import('@/pages/InstallWizard'))
const FleetPage = lazy(() => import('@/pages/FleetPage'))
const ImportPage = lazy(() => import('@/pages/import/ImportPage'))
const SettingsPage = lazy(() => import('@/pages/SettingsPage'))
const SystemCheckPage = lazy(() => import('@/pages/SystemCheckPage'))

// The styleguide only exists in mock builds: the constant is false in production, so this branch (and the page) is dropped.
const Styleguide = __KORA_MOCK__ ? lazy(() => import('@/pages/Styleguide')) : null

export function App({ boot }: { boot: BootResult }) {
  // A change of language redraws everything below: nothing keeps text of the old language.
  const locale = useLocale()
  const client = useMemo(
    () =>
      new QueryClient({
        defaultOptions: {
          queries: { retry: (n, e) => !(e instanceof ApiError && e.status < 500 && e.status !== 0) && n < 2, refetchOnWindowFocus: false, staleTime: 2000 },
        },
      }),
    [],
  )
  // `signed_out`: the quiet Tailscale try did not work; show the sign-in screen without an error.
  const [signedOut, setSignedOut] = useState<ApiError | boolean>(boot.status === 'failed' ? boot.error : boot.status === 'signed_out')
  const [enrolling, setEnrolling] = useState(boot.status === 'enrol' ? boot.link : null)

  useEffect(() => {
    setUnauthorizedHandler(() => setSignedOut((s) => s || true))
    return () => setUnauthorizedHandler(null)
  }, [])

  // An enrolment link opened in a tab that already has the panel open only changes the address's hash (no reload): follow it.
  useEffect(() => {
    const onHash = () => {
      const link = enrolFromHash(window.location.hash)
      if (link) {
        clearHash()
        setEnrolling(link)
      }
    }
    window.addEventListener('hashchange', onHash)
    return () => window.removeEventListener('hashchange', onHash)
  }, [])

  if (enrolling) {
    // The set-up signs the person in at the end; then the panel opens.
    return <Enrol link={enrolling} onDone={() => { client.clear(); setEnrolling(null); setSignedOut(false) }} />
  }

  if (signedOut) {
    return (
      <SignedOut
        onSignedIn={() => {
          client.clear()
          setSignedOut(false)
        }}
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
        <ReauthProvider>
        <AdminActions>
        <BrowserRouter key={locale}>
          <ToolchainActions>
          <Routes>
            {Styleguide && <Route path="styleguide" element={<Suspense fallback={null}><Styleguide /></Suspense>} />}
            <Route element={<AppShell />}>
              <Route index element={<ServersPage />} />
              <Route path="servers/:name" element={<ServerDetailPage />} />
              <Route path="servers/:name/resources/:resource" element={<ResourcePage />} />
              <Route path="install" element={<InstallWizard />} />
              <Route path="import" element={<ImportPage />} />
              <Route path="fleet" element={<FleetPage />} />
              <Route path="settings" element={<SettingsPage />} />
              <Route path="system" element={<SystemCheckPage />} />
              <Route path="*" element={<NotFound />} />
            </Route>
          </Routes>
          </ToolchainActions>
        </BrowserRouter>
        </AdminActions>
        </ReauthProvider>
      </AlertsProvider>
    </QueryClientProvider>
  )
}
