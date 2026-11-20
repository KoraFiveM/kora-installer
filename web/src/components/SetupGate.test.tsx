import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { screen, waitFor } from '@testing-library/react'
import { Route, Routes } from 'react-router-dom'
import InstallWizard from '@/pages/InstallWizard'
import { AppShell } from './AppShell'
import { jsonResponse, renderApp } from '@/test/utils'

const fetchMock = vi.fn()
beforeEach(() => { fetchMock.mockReset(); vi.stubGlobal('fetch', fetchMock) })
afterEach(() => vi.unstubAllGlobals())

const gh = (ready: boolean) => ({ signed_in: ready, user: ready ? 'jay' : null, state: ready ? 'ready' : 'signed_out', missing_scopes: [], problem: null })
const keys = (n: number) => ({ keys: Array.from({ length: n }, (_, i) => ({ name: `k${i}`, last4: 'abcd', added: 0, servers: [], stopped_installs: 0 })), can_manage: true, path: null, keychain: { mode: 'auto', line: 'New keys go to the system keychain.', active: 'secret-service', available: true } })

function answer(github: boolean, saved: number) {
  fetchMock.mockImplementation(async (url: string) => {
    if (url === '/api/v1/github/status') return jsonResponse(200, gh(github))
    if (url === '/api/v1/keys') return jsonResponse(200, keys(saved))
    if (url === '/api/v1/meta') return jsonResponse(200, { version: '1', node: 'n', platform: 'linux', remote: false, ops: [] })
    return jsonResponse(200, { alerts: [], servers: [] })
  })
}
const app = (route: string) => renderApp(<Routes><Route element={<AppShell />}><Route path="/install" element={<InstallWizard />} /><Route path="*" element={<p>page body</p>} /></Route></Routes>, route)

describe('Setup gate (the install wizard)', () => {
  it('shows the checklist instead of the wizard until a Cfx key is saved and GitHub is signed in', async () => {
    answer(false, 0)
    app('/install')
    expect(await screen.findByRole('heading', { name: 'Get set up' })).toBeInTheDocument()
    expect(screen.getAllByText('Still to do')).toHaveLength(2)
  })

  it('names only what is still missing', async () => {
    answer(true, 0)
    app('/install')
    expect(await screen.findByRole('heading', { name: 'Get set up' })).toBeInTheDocument()
    expect(screen.getByText('Done')).toBeInTheDocument()
    expect(screen.getAllByText('Still to do')).toHaveLength(1)
  })

  it('leaves every other page open, so servers can still be added and managed', async () => {
    answer(false, 0)
    app('/')
    expect(await screen.findByText('page body')).toBeInTheDocument()
    expect(screen.queryByRole('heading', { name: 'Get set up' })).toBeNull()
  })

  it('does not show the checklist once both are done', async () => {
    answer(true, 1)
    app('/install')
    await waitFor(() => expect(fetchMock.mock.calls.some(([u]) => u === '/api/v1/keys')).toBe(true))
    await waitFor(() => expect(fetchMock.mock.calls.some(([u]) => u === '/api/v1/github/status')).toBe(true))
    await new Promise((r) => setTimeout(r, 200))
    expect(screen.queryByRole('heading', { name: 'Get set up' })).toBeNull()
  })

  it('never blocks when the answers cannot be read', async () => {
    fetchMock.mockImplementation(async (url: string) => (url.endsWith('/github/status') || url.endsWith('/keys') ? jsonResponse(500, { message: 'x' }) : url.endsWith('/meta') ? jsonResponse(200, { version: '1', node: 'n', platform: 'linux', remote: false, ops: [] }) : jsonResponse(200, { alerts: [] })))
    app('/install')
    await new Promise((r) => setTimeout(r, 300))
    expect(screen.queryByRole('heading', { name: 'Get set up' })).toBeNull()
  })
})
