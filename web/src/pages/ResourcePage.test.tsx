import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { Route, Routes } from 'react-router-dom'
import type { ResourcePage as Page } from '@/api/resource'
import type { UpdatePreview } from '@/api/types'
import { jsonResponse, renderApp } from '@/test/utils'
import ResourcePage from './ResourcePage'

const base = (over: Partial<Page> = {}): Page => ({
  id: 'kora_hud',
  folder: '[kora]/kora_hud',
  repository: 'https://github.com/KoraFiveM/kora_hud.git',
  default_branch: 'dev',
  third_party: false,
  branch: { id: 'kora_hud', folder: 'kora_hud', state: 'default', default_branch: 'dev', label: 'the normal version (dev)', changed_count: 0, stashes: 0 },
  restart: { needed: false },
  server: { name: 'live', mode: 'development', running: true, console_enabled: true, console_waiting_for_restart: false },
  ...over,
})
const preview = (status: 'clean' | 'up_to_date'): UpdatePreview => ({ new_release: null, resources: [{ id: 'kora_hud', from: 'aaaaaaa', to: 'bbbbbbb', behind: status === 'clean' ? 2 : 0, changelog: ['A small fix'], needs_restart: false, status }] })

const fetchMock = vi.fn()
let page: Page
let calls: string[] = []
function install() {
  fetchMock.mockImplementation(async (url: string, init?: RequestInit) => {
    const method = init?.method ?? 'GET'
    calls.push(`${method} ${url.replace(/^.*\/api\/v1/, '')}`)
    if (url.endsWith('/servers/live/resources/kora_hud') && method === 'GET') return jsonResponse(200, page)
    if (url.endsWith('/resources/kora_hud/control')) {
      // A restart of the resource loads the new files: the notice goes.
      page = { ...page, restart: { needed: false } }
      return jsonResponse(200, { message: 'Asked the server to restart kora_hud.' })
    }
    if (url.endsWith('/update/preview')) return jsonResponse(202, { job: 'j1' })
    if (url.endsWith('/update/apply')) return jsonResponse(202, { job: 'j2' })
    if (url.endsWith('/jobs/j1')) return jsonResponse(200, { id: 'j1', op: 'update_preview', state: 'done', events: [], result: preview('clean'), error: null })
    if (url.endsWith('/jobs/j2')) return jsonResponse(200, { id: 'j2', op: 'update_apply', state: 'done', events: [], result: { updated: [{ id: 'kora_hud', from: 'a', to: 'b' }], skipped: [], deferred: [], rolled_back: [], needs_attention: [] }, error: null })
    if (url.endsWith('/servers/live/control')) return jsonResponse(202, { job: 'j3' })
    return jsonResponse(200, {})
  })
}
const show = () => renderApp(<Routes><Route path="/servers/:name/resources/:resource" element={<ResourcePage />} /></Routes>, '/servers/live/resources/kora_hud')

beforeEach(() => { fetchMock.mockReset(); calls = []; page = base(); install(); vi.stubGlobal('fetch', fetchMock); vi.stubGlobal('EventSource', undefined) })
afterEach(() => vi.unstubAllGlobals())

describe('resource page', () => {
  it('shows the resource, where it is and a way back', async () => {
    show()
    expect(await screen.findByRole('heading', { name: 'kora_hud' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: /Resources of live/ })).toHaveAttribute('href', '/servers/live?tab=resources')
    expect(screen.getByText(/KoraFiveM\/kora_hud/)).toBeInTheDocument()
    expect(screen.queryByText(/Restart needed/)).toBeNull()
  })

  it('says a restart is needed, from the server, and clears it after restarting the resource', async () => {
    page = base({ restart: { needed: true, why: 'moved to pull request #12', notice: 'Restart needed: the running server still has the old version.' } })
    const user = userEvent.setup()
    show()
    expect(await screen.findByText('Restart needed: the running server still has the old version.')).toBeInTheDocument()
    expect(screen.getByText(/moved to pull request #12/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: /Restart this resource/ }))
    await waitFor(() => expect(screen.queryByText(/Restart needed/)).toBeNull())
    expect(calls.some((c) => c.startsWith('POST') && c.endsWith('/resources/kora_hud/control'))).toBe(true)
  })

  it('highlights a resource that is not on its normal version', async () => {
    page = base({ branch: { id: 'kora_hud', folder: 'kora_hud', state: 'pr', pr: 12, default_branch: 'dev', label: 'pull request #12', changed_count: 0, stashes: 0 } })
    show()
    expect(await screen.findByText('Trying pull request #12')).toBeInTheDocument()
  })

  it('explains why start/stop/restart are off and offers the fix when the server is stopped', async () => {
    page = base({ server: { name: 'live', mode: 'development', running: false, console_enabled: true, console_waiting_for_restart: false } })
    show()
    expect(await screen.findByText(/The server is not running/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Start' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Start the server' })).toBeEnabled()
  })

  it('points to the Console tab when the live console is off', async () => {
    page = base({ server: { name: 'live', mode: 'development', running: true, console_enabled: false, console_waiting_for_restart: false } })
    show()
    expect(await screen.findByRole('link', { name: 'Open the Console tab' })).toHaveAttribute('href', '/servers/live?tab=console')
    expect(screen.getByRole('button', { name: /Restart$/ })).toBeDisabled()
  })

  it('checks for an update and then updates only this resource, without restarting', async () => {
    const user = userEvent.setup()
    const bodies: unknown[] = []
    const inner = fetchMock.getMockImplementation()!
    fetchMock.mockImplementation(async (url: string, init?: RequestInit) => {
      if (url.endsWith('/update/preview') || url.endsWith('/update/apply')) bodies.push(JSON.parse(String(init?.body)))
      return inner(url, init)
    })
    show()
    await user.click(await screen.findByRole('button', { name: /Check for update/ }))
    expect(await screen.findByText('Ready to update')).toBeInTheDocument()
    expect(screen.getByText(/2 newer changes available/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: /^Update$/ }))
    await screen.findByText(/Updated\. The running server still has the old version/)
    expect(bodies[0]).toEqual({ scope: { resources: ['kora_hud'] } })
    expect(bodies[1]).toMatchObject({ scope: { resources: ['kora_hud'] }, restart: 'none', targets: { kora_hud: 'bbbbbbb' } })
  })

  it('shows a plain page when the resource is not on the server', async () => {
    fetchMock.mockImplementation(async () => jsonResponse(404, { error: { code: 'not_found', message: 'nope' } }))
    show()
    await waitFor(() => expect(screen.queryByRole('heading', { name: 'kora_hud' })).toBeNull())
  })
})
