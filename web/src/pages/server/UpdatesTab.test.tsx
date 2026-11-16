import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { mergeRows, reportHasProblems, shortSha, UpdatesTab } from './UpdatesTab'
import { timeAgo } from '@/lib/ago'
import type { ResourceChange, ServerDetail, UpdatePreview, UpdateReport } from '@/api/types'
import { jsonResponse, renderApp } from '@/test/utils'

const row = (id: string, status: ResourceChange['status'], behind = 1): ResourceChange => ({ id, from: 'aaaaaaa', to: 'bbbbbbb', behind, changelog: [], needs_restart: false, status })
const all: UpdatePreview = { new_release: null, resources: [row('kora_core', 'clean'), row('kora_hud', 'clean'), row('kora_chat', 'up_to_date', 0)] }
const server = { name: 'dev' } as unknown as ServerDetail

const fetchMock = vi.fn()
let bodies: unknown[] = []
// Jobs answer on the first poll: the preview job id decides the result.
function install(results: Record<string, UpdatePreview>) {
  let n = 0
  fetchMock.mockImplementation(async (url: string, init?: RequestInit) => {
    if (url.endsWith('/update/preview')) {
      bodies.push(JSON.parse(String(init?.body)))
      return jsonResponse(202, { job: `j${++n}` })
    }
    const m = /\/jobs\/(j\d+)$/.exec(url)
    if (m) return jsonResponse(200, { id: m[1], op: 'update_preview', state: 'done', events: [], result: results[m[1]], error: null })
    if (url.includes('/snapshots')) return jsonResponse(200, [])
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

describe('UpdatesTab row refresh', () => {
  it('checks one resource, updates only that row and keeps the selection', async () => {
    const user = userEvent.setup()
    install({ j1: all, j2: { new_release: null, resources: [row('kora_hud', 'up_to_date', 0)] } })
    renderApp(<UpdatesTab server={server} />)
    await user.click(screen.getByRole('button', { name: 'Check for updates' }))
    await screen.findByText('kora_core')
    expect(screen.getByRole('button', { name: /Update 2 resources/ })).toBeEnabled()
    expect(screen.getAllByText(/Last checked just now/)).toHaveLength(3)

    await user.click(screen.getByRole('button', { name: 'Check kora_hud' }))
    await waitFor(() => expect(bodies).toHaveLength(2))
    expect(bodies[1]).toEqual({ scope: { resources: ['kora_hud'] } })
    // hud is now up to date and leaves the selection; core stays checked.
    await screen.findByRole('button', { name: /Update 1 resource$/ })
    expect(screen.getByLabelText('Update kora_core')).toBeChecked()
    const hud = screen.getByLabelText('Update kora_hud')
    expect(hud).toBeDisabled()
    expect(within(hud.closest('li') as HTMLElement).getByText('Up to date')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Check kora_core' })).toBeEnabled()
  })

  it('disables every Check button while a job runs', async () => {
    const user = userEvent.setup()
    install({ j1: all })
    let release: (r: Response) => void = () => {}
    const base = fetchMock.getMockImplementation()!
    fetchMock.mockImplementation((url: string, init?: RequestInit) => (/\/jobs\/j2$/.test(url) ? new Promise<Response>((res) => { release = res }) : base(url, init)))
    renderApp(<UpdatesTab server={server} />)
    await user.click(screen.getByRole('button', { name: 'Check for updates' }))
    await screen.findByText('kora_core')
    await user.click(screen.getByRole('button', { name: 'Check kora_core' }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Check kora_hud' })).toBeDisabled())
    expect(screen.getByRole('button', { name: 'Check kora_core' })).toHaveAttribute('aria-busy', 'true')
    release(jsonResponse(200, { id: 'j2', op: 'update_preview', state: 'done', events: [], result: { new_release: null, resources: [row('kora_core', 'clean')] }, error: null }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Check kora_hud' })).toBeEnabled())
  })
})

const sha = (c: string) => c.repeat(40).slice(0, 40)
const report = (over: Partial<UpdateReport> = {}): UpdateReport => ({ server: 'dev', updated: [{ id: 'kora_core', from: sha('a'), to: sha('b') }], skipped: [], deferred: [], rolled_back: [], sql_flagged: [], needs_attention: [], forks_synced: [], forks_left_alone: [], ...over })

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
      if (url.includes('/snapshots')) return jsonResponse(200, { snapshots: [] })
      return jsonResponse(200, {})
    })
    renderApp(<UpdatesTab server={server} />)
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
