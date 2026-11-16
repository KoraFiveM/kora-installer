import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { screen } from '@testing-library/react'
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
