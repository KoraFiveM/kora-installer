import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { screen } from '@testing-library/react'
import { Route, Routes } from 'react-router-dom'
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
    if (url === '/api/v1/meta') return jsonResponse(200, { version: '1', node: 'n', platform: 'linux', remote: false })
    return jsonResponse(200, { alerts: [], servers: [] })
  })
}
const shell = (route: string) => renderApp(<Routes><Route element={<AppShell />}><Route path="*" element={<p>page body</p>} /></Route></Routes>, route)

describe('Setup gate', () => {
  it('shows the checklist instead of the page until a Cfx key is saved and GitHub is signed in', async () => {
    answer(false, 0)
    shell('/')
    expect(await screen.findByRole('heading', { name: 'Get set up' })).toBeInTheDocument()
    expect(screen.queryByText('page body')).toBeNull()
    expect(screen.getAllByText('Still to do')).toHaveLength(2)
  })

  it('names only what is still missing', async () => {
    answer(true, 0)
    shell('/')
    expect(await screen.findByRole('heading', { name: 'Get set up' })).toBeInTheDocument()
    expect(screen.getByText('Done')).toBeInTheDocument()
    expect(screen.getAllByText('Still to do')).toHaveLength(1)
  })

  it('lets Settings through, where both are done', async () => {
    answer(false, 0)
    shell('/settings')
    expect(await screen.findByText('page body')).toBeInTheDocument()
    expect(screen.queryByRole('heading', { name: 'Get set up' })).toBeNull()
  })

  it('shows the page once both are done', async () => {
    answer(true, 1)
    shell('/')
    expect(await screen.findByText('page body')).toBeInTheDocument()
    expect(screen.queryByRole('heading', { name: 'Get set up' })).toBeNull()
  })

  it('never locks the panel when the answers cannot be read', async () => {
    fetchMock.mockImplementation(async (url: string) => (url.endsWith('/github/status') || url.endsWith('/keys') ? jsonResponse(500, { message: 'x' }) : jsonResponse(200, { alerts: [] })))
    shell('/')
    expect(await screen.findByText('page body')).toBeInTheDocument()
  })
})
