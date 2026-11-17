import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { reportHasProblems, shortSha } from './UpdatesPanel'
import { mergeRows } from './useUpdates'
import { ResourcesTab } from './ResourcesTab'
import { timeAgo } from '@/lib/ago'
import type { ResourceChange, ServerDetail, UpdatePreview, UpdateReport } from '@/api/types'
import { jsonResponse, renderApp } from '@/test/utils'

const row = (id: string, status: ResourceChange['status'], behind = 1): ResourceChange => ({ id, from: 'aaaaaaa', to: 'bbbbbbb', behind, changelog: [], needs_restart: false, status })
const all: UpdatePreview = { new_release: null, resources: [row('kora_core', 'clean'), row('kora_hud', 'clean'), row('kora_chat', 'up_to_date', 0)] }
const server = { name: 'dev', mode: 'production', running: true, console_enabled: true } as unknown as ServerDetail

const fetchMock = vi.fn()
let bodies: unknown[] = []
// Jobs answer on the first poll: the preview job id decides the result.
function install(results: Record<string, UpdatePreview>, installed: string[] = []) {
  let n = 0
  fetchMock.mockImplementation(async (url: string, init?: RequestInit) => {
    if (url.endsWith('/update/preview')) {
      bodies.push(JSON.parse(String(init?.body)))
      return jsonResponse(202, { job: `j${++n}` })
    }
    const m = /\/jobs\/(j\d+)$/.exec(url)
    if (m) return jsonResponse(200, { id: m[1], op: 'update_preview', state: 'done', events: [], result: results[m[1]], error: null })
    if (url.endsWith('/resources')) return jsonResponse(200, { resources: installed })
    if (url.endsWith('/catalog')) return jsonResponse(200, { catalog: { resources: [] }, installed: [] })
    if (url.includes('/snapshots')) return jsonResponse(200, { snapshots: [] })
    return jsonResponse(200, {})
  })
}
beforeEach(() => { fetchMock.mockReset(); bodies = []; vi.stubGlobal('fetch', fetchMock); vi.stubGlobal('EventSource', undefined) })
afterEach(() => vi.unstubAllGlobals())

describe('mergeRows', () => {
  it('replaces only the named rows and keeps order', () => {
    const m = mergeRows(all, [row('kora_hud', 'up_to_date', 0)])
    expect(m.resources.map((r) => [r.id, r.status])).toEqual([['kora_core', 'clean'], ['kora_hud', 'up_to_date'], ['kora_chat', 'up_to_date']])
    expect(m.resources[0]).toBe(all.resources[0])
  })
  it('adds a row that was not listed', () => expect(mergeRows(all, [row('new', 'clean')]).resources).toHaveLength(4))
})

describe('timeAgo', () => {
  it('speaks plainly', () => {
    expect(timeAgo(0, 10_000)).toBe('just now')
    expect(timeAgo(0, 120_000)).toBe('2 minutes ago')
    expect(timeAgo(0, 3_600_000)).toBe('1 hour ago')
    expect(timeAgo(0, 2 * 86_400_000)).toBe('2 days ago')
  })
})

describe('Updates panel row refresh', () => {
  it('checks one resource, updates only that row and keeps the selection', async () => {
    const user = userEvent.setup()
    install({ j1: all, j2: { new_release: null, resources: [row('kora_hud', 'up_to_date', 0)] } })
    renderApp(<ResourcesTab server={server} />)
    await user.click(screen.getByRole('button', { name: 'Check for updates' }))
    await screen.findByText('kora_core')
    expect(screen.getByRole('button', { name: /Update 2 resources/ })).toBeEnabled()
    // Only what needs a decision is listed; the rest is counted.
    expect(screen.getAllByText(/Last checked just now/)).toHaveLength(2)
    expect(screen.getByText('1 other resource is up to date.')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Check kora_hud' }))
    await waitFor(() => expect(bodies).toHaveLength(2))
    expect(bodies[1]).toEqual({ scope: { resources: ['kora_hud'] } })
    // hud is now up to date and leaves the selection; core stays checked.
    await screen.findByRole('button', { name: /Update 1 resource$/ })
    expect(screen.getByLabelText('Update kora_core')).toBeChecked()
    expect(screen.queryByLabelText('Update kora_hud')).not.toBeInTheDocument()
    expect(screen.getByText('2 other resources are up to date.')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Check kora_core' })).toBeEnabled()
  })

  it('disables every Check button while a job runs', async () => {
    const user = userEvent.setup()
    install({ j1: all })
    let release: (r: Response) => void = () => {}
    const base = fetchMock.getMockImplementation()!
    fetchMock.mockImplementation((url: string, init?: RequestInit) => (/\/jobs\/j2$/.test(url) ? new Promise<Response>((res) => { release = res }) : base(url, init)))
    renderApp(<ResourcesTab server={server} />)
    await user.click(screen.getByRole('button', { name: 'Check for updates' }))
    await screen.findByText('kora_core')
    await user.click(screen.getByRole('button', { name: 'Check kora_core' }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Check kora_hud' })).toBeDisabled())
    expect(screen.getByRole('button', { name: 'Check kora_core' })).toHaveAttribute('aria-busy', 'true')
    release(jsonResponse(200, { id: 'j2', op: 'update_preview', state: 'done', events: [], result: { new_release: null, resources: [row('kora_core', 'clean')] }, error: null }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Check kora_hud' })).toBeEnabled())
  })
})

describe('installed resources', () => {
  it('checks one resource from its row and shows what was found beside the name', async () => {
    const user = userEvent.setup()
    install({ j1: { new_release: null, resources: [row('kora_core', 'clean')] } }, ['kora_core', 'kora_chat'])
    renderApp(<ResourcesTab server={server} />)
    await user.click(await screen.findByRole('button', { name: 'Check kora_core for updates' }))
    expect(bodies).toEqual([{ scope: { resources: ['kora_core'] } }])
    const item = (await screen.findByRole('button', { name: 'Start kora_core' })).closest('li') as HTMLElement
    expect(await within(item).findByText('Ready to update')).toBeInTheDocument()
    // The same result is offered in the Updates card, ticked and ready.
    expect(screen.getByRole('button', { name: /Update 1 resource$/ })).toBeEnabled()
    // One resource checked is not "everything".
    expect(screen.queryByText('Everything is up to date.')).not.toBeInTheDocument()
  })

  it('says everything is up to date only after a full check', async () => {
    const user = userEvent.setup()
    const chat = { new_release: null, resources: [row('kora_chat', 'up_to_date', 0)] }
    install({ j1: chat, j2: chat }, ['kora_chat'])
    renderApp(<ResourcesTab server={server} />)
    await user.click(await screen.findByRole('button', { name: 'Check kora_chat for updates' }))
    expect(await screen.findByText('Up to date')).toBeInTheDocument()
    expect(screen.queryByText('Everything is up to date.')).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Check again' }))
    expect(await screen.findByText('Everything is up to date.')).toBeInTheDocument()
  })

  it('keeps what a check found when the tab is closed and opened again', async () => {
    const user = userEvent.setup()
    install({ j1: all }, ['kora_core'])
    const first = renderApp(<ResourcesTab server={server} />)
    await user.click(screen.getByRole('button', { name: 'Check for updates' }))
    await screen.findByRole('button', { name: /Update 2 resources/ })
    await user.click(screen.getByLabelText('Update kora_hud'))
    first.unmount()

    renderApp(<ResourcesTab server={server} />)
    expect(screen.getByRole('button', { name: /Update 1 resource$/ })).toBeEnabled()
    expect(screen.getByLabelText('Update kora_core')).toBeChecked()
    expect(screen.getByLabelText('Update kora_hud')).not.toBeChecked()
    // Nothing was asked again.
    expect(bodies).toHaveLength(1)
    expect(fetchMock.mock.calls.filter(([u]) => /\/jobs\/j1$/.test(String(u)))).toHaveLength(1)
  })

  it('still follows a check that was running when the tab was closed, without starting another', async () => {
    const user = userEvent.setup()
    install({ j1: all })
    let release: (r: Response) => void = () => {}
    const base = fetchMock.getMockImplementation()!
    let held = true
    fetchMock.mockImplementation((url: string, init?: RequestInit) => (held && /\/jobs\/j1$/.test(url) ? new Promise<Response>((res) => { release = res }) : base(url, init)))
    const first = renderApp(<ResourcesTab server={server} />)
    await user.click(screen.getByRole('button', { name: 'Check for updates' }))
    await screen.findByRole('button', { name: 'Checking...' })
    first.unmount()
    held = false
    release(jsonResponse(200, { id: 'j1', op: 'update_preview', state: 'running', events: [], result: null, error: null }))

    renderApp(<ResourcesTab server={server} />)
    expect(await screen.findByRole('button', { name: /Update 2 resources/ })).toBeEnabled()
    expect(bodies).toHaveLength(1)
  })

  it('keeps another server out of it', async () => {
    const user = userEvent.setup()
    install({ j1: all })
    const first = renderApp(<ResourcesTab server={server} />)
    await user.click(screen.getByRole('button', { name: 'Check for updates' }))
    await screen.findByRole('button', { name: /Update 2 resources/ })
    first.unmount()
    renderApp(<ResourcesTab server={{ ...server, name: 'other' }} />)
    expect(screen.getByRole('button', { name: 'Check for updates' })).toBeEnabled()
    expect(screen.queryByText('kora_core')).not.toBeInTheDocument()
  })
})

describe('start, stop and restart a resource', () => {
  const calls: { url: string; body: unknown }[] = []
  function withControl(answer: () => Response = () => jsonResponse(200, { message: 'Asked the server to do it.' })) {
    calls.length = 0
    install({}, ['kora_core', 'kora_console'])
    const base = fetchMock.getMockImplementation()!
    fetchMock.mockImplementation(async (url: string, init?: RequestInit) => {
      if (url.endsWith('/control')) {
        calls.push({ url, body: JSON.parse(String(init?.body)) })
        return answer()
      }
      return base(url, init)
    })
  }

  it('starts at once and says what the panel answered', async () => {
    const user = userEvent.setup()
    withControl()
    renderApp(<ResourcesTab server={server} />)
    await user.click(await screen.findByRole('button', { name: 'Start kora_core' }))
    expect(await screen.findByText('Asked the server to do it.')).toBeInTheDocument()
    expect(calls).toEqual([{ url: '/api/v1/servers/dev/resources/kora_core/control', body: { action: 'start' } }])
  })

  it('asks before stopping or restarting, and does nothing on Cancel', async () => {
    const user = userEvent.setup()
    withControl()
    renderApp(<ResourcesTab server={server} />)
    await user.click(await screen.findByRole('button', { name: 'Stop kora_core' }))
    const dialog = await screen.findByRole('dialog')
    expect(within(dialog).getByText('Stop kora_core?')).toBeInTheDocument()
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }))
    expect(calls).toHaveLength(0)

    await user.click(screen.getByRole('button', { name: 'Restart kora_core' }))
    await user.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Restart resource' }))
    await waitFor(() => expect(calls).toHaveLength(1))
    expect(calls[0].body).toEqual({ action: 'restart' })
  })

  it('shows why it did not work under the resource', async () => {
    const user = userEvent.setup()
    withControl(() => jsonResponse(409, { error: { code: 'conflict', message: 'Another update is running for this server.' } }))
    renderApp(<ResourcesTab server={server} />)
    await user.click(await screen.findByRole('button', { name: 'Start kora_core' }))
    expect(await screen.findByText('Another update is running for this server.')).toBeInTheDocument()
  })

  it('never offers to stop the resource Kora talks through', async () => {
    withControl()
    renderApp(<ResourcesTab server={server} />)
    expect(await screen.findByRole('button', { name: 'Stop kora_console' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Restart kora_console' })).toBeEnabled()
  })

  it('is off, with the reason, while the server is stopped or the console is not on', async () => {
    withControl()
    const stopped = renderApp(<ResourcesTab server={{ ...server, running: false }} />)
    expect(await screen.findByRole('button', { name: 'Start kora_core' })).toBeDisabled()
    expect(screen.getByText(/The server is not running/)).toBeInTheDocument()
    // Checking for updates does not need a running server.
    expect(screen.getByRole('button', { name: 'Check kora_core for updates' })).toBeEnabled()
    stopped.unmount()
    renderApp(<ResourcesTab server={{ ...server, console_enabled: false }} />)
    expect(await screen.findByRole('button', { name: 'Restart kora_core' })).toBeDisabled()
    expect(screen.getByText(/Turn on the live console/)).toBeInTheDocument()
  })
})

const sha = (c: string) => c.repeat(40).slice(0, 40)
const report = (over: Partial<UpdateReport> = {}): UpdateReport => ({ server: 'dev', updated: [{ id: 'kora_core', from: sha('a'), to: sha('b') }], skipped: [], deferred: [], rolled_back: [], sql_flagged: [], needs_attention: [], forks_synced: [], forks_left_alone: [], not_attempted: [], held: [], notes: [], ...over })

describe('update results', () => {
  it('short commit ids and what counts as a problem', () => {
    expect(shortSha(sha('a'))).toBe('aaaaaaa')
    expect(shortSha('kora')).toBe('kora')
    expect(reportHasProblems(report())).toBe(false)
    expect(reportHasProblems(report({ not_attempted: ['x'] }))).toBe(true)
    expect(reportHasProblems(report({ rolled_back: [['x', 'boot failed']] }))).toBe(true)
    expect(reportHasProblems(report({ held: [['x', 'held']] }))).toBe(true)
  })

  async function applyWith(result: UpdateReport) {
    const user = userEvent.setup()
    const applied: unknown[] = []
    const preview: UpdatePreview = { new_release: null, notes: ['the online release list could not be read'], resources: [{ ...row('kora_core', 'clean'), from: sha('a'), to: sha('b') }] }
    fetchMock.mockImplementation(async (url: string, init?: RequestInit) => {
      if (url.endsWith('/update/preview')) return jsonResponse(202, { job: 'p1' })
      if (url.endsWith('/update/apply')) {
        applied.push(JSON.parse(String(init?.body)))
        return jsonResponse(202, { job: 'a1' })
      }
      if (url.endsWith('/jobs/p1')) return jsonResponse(200, { id: 'p1', op: 'update_preview', state: 'done', events: [], result: preview, error: null })
      if (url.endsWith('/jobs/a1')) return jsonResponse(200, { id: 'a1', op: 'update_apply', state: 'done', events: [], result, error: null })
      if (url.endsWith('/resources')) return jsonResponse(200, { resources: [] })
      if (url.endsWith('/catalog')) return jsonResponse(200, { catalog: { resources: [] }, installed: [] })
      if (url.includes('/snapshots')) return jsonResponse(200, { snapshots: [] })
      return jsonResponse(200, {})
    })
    renderApp(<ResourcesTab server={server} />)
    await user.click(screen.getByRole('button', { name: 'Check for updates' }))
    await screen.findByText('kora_core')
    expect(screen.getByText('the online release list could not be read')).toBeInTheDocument()
    expect(screen.getByText('aaaaaaa')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: /Update 1 resource$/ }))
    await waitFor(() => expect(applied).toHaveLength(1))
    return applied[0] as { targets: Record<string, string>; restart: string }
  }

  it('applies exactly the previewed versions and says plainly when all went well', async () => {
    const body = await applyWith(report())
    expect(body.targets).toEqual({ kora_core: sha('b') })
    expect(body.restart).toBe('when_quiet')
    expect(await screen.findByText('Update finished')).toBeInTheDocument()
    expect(screen.getByText('kora_core (aaaaaaa to bbbbbbb)')).toBeInTheDocument()
  })

  it('warns instead of a green "finished" when something was rolled back or not attempted', async () => {
    await applyWith(report({ updated: [], rolled_back: [['kora_core', 'boot failed']], not_attempted: ['kora_hud'] }))
    expect(await screen.findByText('Update finished, but some things need a look')).toBeInTheDocument()
    expect(screen.queryByText('Update finished')).not.toBeInTheDocument()
    expect(screen.getByText('kora_hud')).toBeInTheDocument()
  })
})
