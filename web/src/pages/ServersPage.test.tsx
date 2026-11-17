import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { screen } from '@testing-library/react'
import { Route, Routes } from 'react-router-dom'
import type { ServerSummary } from '@/api/types'
import ServersPage from './ServersPage'
import { jsonResponse, renderApp } from '@/test/utils'

const fetchMock = vi.fn()
beforeEach(() => { fetchMock.mockReset(); vi.stubGlobal('fetch', fetchMock) })
afterEach(() => vi.unstubAllGlobals())

const srv = (name: string, badge: ServerSummary['badge']): ServerSummary => ({ name, dir: `/srv/${name}`, mode: 'production', ports: { game: 30120, txadmin: 40120 }, created_unix: 0, status: 'stopped', badge, managed: true })

describe('ServersPage', () => {
  it('links the update chip to the Resources tab of that server, when there is something to look at', async () => {
    fetchMock.mockImplementation(async (url: string) =>
      url === '/api/v1/servers' ? jsonResponse(200, { servers: [srv('my server', { kind: 'updates', count: 2 }), srv('quiet', { kind: 'up_to_date' })] }) : jsonResponse(200, { alerts: [] }),
    )
    renderApp(<ServersPage />)
    expect(await screen.findByRole('link', { name: '2 updates' })).toHaveAttribute('href', '/servers/my%20server?tab=resources')
    expect(screen.getByText('Up to date').closest('a')).toBeNull()
  })
})

describe('ServersPage on a first visit', () => {
  it('goes straight to the install wizard when there are no servers, once per tab', async () => {
    sessionStorage.clear()
    fetchMock.mockImplementation(async (url: string) => (url === '/api/v1/servers' ? jsonResponse(200, { servers: [] }) : jsonResponse(200, { alerts: [] })))
    renderApp(<Routes><Route path="/" element={<ServersPage />} /><Route path="/install" element={<p>wizard here</p>} /></Routes>)
    expect(await screen.findByText('wizard here')).toBeInTheDocument()
    expect(sessionStorage.getItem('kora.firstRunShown')).toBe('1')
  })

  it('does not send the person back to the wizard after they have seen it', async () => {
    sessionStorage.setItem('kora.firstRunShown', '1')
    fetchMock.mockImplementation(async (url: string) => (url === '/api/v1/servers' ? jsonResponse(200, { servers: [] }) : jsonResponse(200, { alerts: [] })))
    renderApp(<Routes><Route path="/" element={<ServersPage />} /><Route path="/install" element={<p>wizard here</p>} /></Routes>)
    expect(await screen.findByText('No servers yet')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /add an existing server/i })).toBeInTheDocument()
  })
})
