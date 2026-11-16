import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { AssetPlan, PaidAssets, ServerDetail } from '@/api/types'
import { PaidAssetsTab, fileSize } from './PaidAssetsTab'
import { jsonResponse, renderApp } from '@/test/utils'

const fetchMock = vi.fn()
beforeEach(() => { fetchMock.mockReset(); vi.stubGlobal('fetch', fetchMock); vi.stubGlobal('EventSource', undefined) })
afterEach(() => vi.unstubAllGlobals())

const server = { name: 'prod', mode: 'production', status: 'stopped' } as unknown as ServerDetail

const listed: PaidAssets = {
  portal_url: 'https://portal.cfx.re/assets/granted-assets',
  assets: [{ name: 'qs-housing', path: '[paid]/qs-housing', version: '3.2.1', imported_unix: 1_700_000_000, present: true, has_fxap: true, backups: 1 }],
  hints: [{ kind: 'entitlement', resource: 'jim-mechanic', line: 'You lack the required entitlement to use jim-mechanic', fix: 'Your licence key belongs to a different Cfx.re account than the one that bought jim-mechanic.' }],
}

const plan = (over: Partial<AssetPlan> = {}): AssetPlan => ({
  zip_name: 'jim-mechanic.zip',
  resources: [{ name: 'jim-mechanic', zip_path: 'jim-mechanic', has_fxmanifest: true, has_fxap: true, escrowed: true, size: 2048, files: 10, version: '1.4', replaces: false }],
  problems: [],
  total_size: 2048,
  can_import: true,
  ...over,
})

/** Answers by URL and method; anything else is a test failure. */
function backend(routes: Record<string, (init: RequestInit) => Response>) {
  fetchMock.mockImplementation(async (url: string, init: RequestInit = {}) => {
    const key = `${init.method ?? 'GET'} ${url}`
    const h = routes[key]
    if (!h) throw new Error(`unexpected ${key}`)
    return h(init)
  })
}

describe('PaidAssetsTab', () => {
  it('links to the Cfx.re Portal and explains the licence rule', async () => {
    backend({ 'GET /api/v1/servers/prod/assets': () => jsonResponse(200, listed), 'GET /api/v1/assets/downloads': () => jsonResponse(200, { found: true, zips: [] }) })
    renderApp(<PaidAssetsTab server={server} />)
    const link = screen.getByRole('link', { name: /Open Cfx.re Portal/ })
    expect(link).toHaveAttribute('href', 'https://portal.cfx.re/assets/granted-assets')
    expect(link).toHaveAttribute('target', '_blank')
    expect(link.getAttribute('rel')).toContain('noopener')
    expect(screen.getByText(/same Cfx.re account that owns this server's licence key/)).toBeInTheDocument()
    expect(await screen.findByText('qs-housing')).toBeInTheDocument()
    expect(screen.getByText('Version 3.2.1')).toBeInTheDocument()
    expect(screen.getByText(/Licence key and asset owner do not match: jim-mechanic/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Replace with newer zip' })).toBeInTheDocument()
    expect(await screen.findByText(/No zip files were downloaded in the last 24 hours/)).toBeInTheDocument()
  })

  it('uploads a chosen zip, shows the plan and imports it', async () => {
    const user = userEvent.setup()
    let uploaded: RequestInit | null = null
    backend({
      'GET /api/v1/servers/prod/assets': () => jsonResponse(200, { ...listed, hints: [] }),
      'GET /api/v1/assets/downloads': () => jsonResponse(200, { found: true, zips: [] }),
      'POST /api/v1/servers/prod/assets/inspect': (init) => { uploaded = init; return jsonResponse(200, { plan: plan(), upload: 'abc123abc123abc123' }) },
      'POST /api/v1/servers/prod/assets': (init) => { expect(JSON.parse(String(init.body))).toEqual({ upload: 'abc123abc123abc123', replace: true }); return jsonResponse(202, { job: 'j1' }) },
      'GET /api/v1/jobs/j1': () => jsonResponse(200, { id: 'j1', op: 'asset_import', server: 'prod', state: 'done', events: [], result: { zip_name: 'jim-mechanic.zip', imported: [{ name: 'jim-mechanic', version: '1.4', replaced: false, backup: null }], ensured: ['jim-mechanic'], notes: ['Restart the server to load the new files.'] }, error: null }),
    })
    renderApp(<PaidAssetsTab server={server} />)
    const file = new File(['PK\u0003\u0004'], 'jim mechanic.zip', { type: 'application/zip' })
    await user.upload(screen.getByLabelText('Choose a zip file'), file)
    const planView = await screen.findByLabelText('What will be imported')
    expect(within(planView).getByText('jim-mechanic')).toBeInTheDocument()
    expect(within(planView).getByText('.fxap file present')).toBeInTheDocument()
    const headers = uploaded!.headers as Record<string, string>
    expect(headers['Content-Type']).toBe('application/zip')
    expect(headers['X-File-Name']).toBe('jim%20mechanic.zip')
    expect(uploaded!.body).toBe(file)
    await user.click(screen.getByRole('button', { name: 'Import 1 asset' }))
    expect(await screen.findByText('Restart the server to load the new files.')).toBeInTheDocument()
    expect(screen.getByText(/ensure jim-mechanic/)).toBeInTheDocument()
  })

  it('shows blocking problems and offers no import', async () => {
    const user = userEvent.setup()
    backend({
      'GET /api/v1/servers/prod/assets': () => jsonResponse(200, { ...listed, hints: [] }),
      'GET /api/v1/assets/downloads': () => jsonResponse(200, { found: true, zips: [{ name: 'bomb.zip', size: 1000, modified_unix: Math.floor(Date.now() / 1000) - 60 }] }),
      'POST /api/v1/servers/prod/assets/inspect': (init) => {
        expect(JSON.parse(String(init.body))).toEqual({ download: 'bomb.zip' })
        return jsonResponse(200, { plan: plan({ can_import: false, problems: [{ kind: 'zip_bomb', resource: null, message: 'This looks like a zip bomb.', blocking: true }] }), download: 'bomb.zip' })
      },
    })
    renderApp(<PaidAssetsTab server={server} />)
    await user.click(await screen.findByRole('button', { name: 'Check this zip' }))
    expect(await screen.findByText("This zip can't be imported")).toBeInTheDocument()
    expect(screen.getByText('This looks like a zip bomb.')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /^Import/ })).not.toBeInTheDocument()
  })

  it('removes an asset after confirmation', async () => {
    const user = userEvent.setup()
    let gone = false
    backend({
      'GET /api/v1/servers/prod/assets': () => jsonResponse(200, gone ? { ...listed, assets: [], hints: [] } : { ...listed, hints: [] }),
      'GET /api/v1/assets/downloads': () => jsonResponse(200, { found: false, zips: [] }),
      'GET /api/v1/servers/prod/resources': () => jsonResponse(200, { resources: [] }),
      'DELETE /api/v1/servers/prod/assets/qs-housing': () => { gone = true; return jsonResponse(200, { notes: ['qs-housing was moved to .kora/asset-backups (kept as a backup).'] }) },
    })
    renderApp(<PaidAssetsTab server={server} />)
    await user.click(await screen.findByRole('button', { name: 'Remove' }))
    const dialog = await screen.findByRole('dialog')
    await user.click(within(dialog).getByRole('button', { name: 'Remove' }))
    expect(await screen.findByText(/kept as a backup/)).toBeInTheDocument()
    await waitFor(() => expect(screen.getByText('No paid assets have been imported yet.')).toBeInTheDocument())
  })

  it('formats sizes', () => {
    expect(fileSize(500)).toBe('500 bytes')
    expect(fileSize(2048)).toBe('2.0 KB')
    expect(fileSize(3 * 1024 * 1024 * 1024)).toBe('3.0 GB')
  })
})
