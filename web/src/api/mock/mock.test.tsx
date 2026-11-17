import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { renderApp } from '@/test/utils'
import ServersPage from '@/pages/ServersPage'
import SettingsPage from '@/pages/SettingsPage'
import Styleguide from '@/pages/Styleguide'
import { installMock } from './index'
import { SCENARIOS, STORAGE_KEY, resolveScenario } from './scenarios'
import type { GithubStatus, Job, ServerSummary } from '../types'

const realFetch = window.fetch
const realEs = window.EventSource
beforeEach(() => sessionStorage.clear())
afterEach(() => {
  window.fetch = realFetch
  window.EventSource = realEs
})

const get = async (path: string) => fetch(`/api/v1${path}`)
const servers = async () => ((await (await get('/servers')).json()) as { servers: ServerSummary[] }).servers

describe('scenario selection', () => {
  it('uses the query parameter and remembers it', () => {
    expect(resolveScenario('?scenario=fleet', sessionStorage)).toBe('fleet')
    expect(sessionStorage.getItem(STORAGE_KEY)).toBe('fleet')
  })
  it('falls back to the remembered scenario, then to default', () => {
    expect(resolveScenario('', sessionStorage)).toBe('default')
    sessionStorage.setItem(STORAGE_KEY, 'trouble')
    expect(resolveScenario('?other=1', sessionStorage)).toBe('trouble')
  })
  it('ignores unknown names and lets a new query win over the remembered one', () => {
    sessionStorage.setItem(STORAGE_KEY, 'empty')
    expect(resolveScenario('?scenario=nope', sessionStorage)).toBe('empty')
    expect(resolveScenario('?scenario=slow', sessionStorage)).toBe('slow')
    expect(resolveScenario('?scenario=slow', null)).toBe('slow')
  })
  it('installMock reads the page URL when no scenario is given', () => {
    sessionStorage.setItem(STORAGE_KEY, 'empty')
    expect(installMock({ speed: 0 })).toBe('empty')
  })
})

describe('scenarios', () => {
  it('default has the three servers', async () => {
    installMock({ scenario: 'default', speed: 0 })
    expect((await servers()).map((s) => s.name)).toEqual(['kora-live', 'kora-dev', 'kora-old'])
  })

  it('fleet covers every mode x status x badge, long names, high ports and an adopted server (deterministically)', async () => {
    installMock({ scenario: 'fleet', speed: 0 })
    const a = await servers()
    installMock({ scenario: 'fleet', speed: 0 })
    expect(await servers()).toEqual(a)
    expect(a.length).toBe(31)
    expect(new Set(a.map((s) => s.name)).size).toBe(a.length)
    const combos = new Set(a.filter((s) => s.managed).map((s) => `${s.mode}/${s.status}/${s.badge?.kind ?? 'none'}`))
    expect(combos.size).toBe(2 * 3 * 5)
    expect(a.some((s) => s.name.length === 32)).toBe(true)
    expect(a.some((s) => s.ports.game > 60000)).toBe(true)
    expect(a.filter((s) => !s.managed)).toHaveLength(1)
  })

  it('offline answers 401 and down rejects', async () => {
    installMock({ scenario: 'offline', speed: 0 })
    expect((await get('/servers')).status).toBe(401)
    installMock({ scenario: 'down', speed: 0 })
    await expect(get('/servers')).rejects.toThrow()
  })

  it('empty shows the first-run screen', async () => {
    installMock({ scenario: 'empty', speed: 0 })
    renderApp(<ServersPage />)
    expect(await screen.findByText('No servers yet')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /install your first server/i })).toBeInTheDocument()
    const d = (await (await get('/installs/defaults')).json()) as { saved: unknown }
    expect(d.saved).toBeNull()
  })

  it('trouble shows the command to run as administrator (with sudo on Linux)', async () => {
    installMock({ scenario: 'trouble', speed: 0 })
    renderApp(<SettingsPage />)
    await userEvent.setup().click(await screen.findByRole('button', { name: 'Tidy up old shortcuts' }))
    const cmd = await screen.findByLabelText('Command to run')
    await waitFor(() => expect(cmd).toHaveTextContent('sudo kora app uninstall --yes'))
    expect(screen.getAllByText(/needs administrator rights/i).length).toBeGreaterThan(0)
  })

  it('trouble: start fails with a port conflict job error, webhook test fails, logs are empty', async () => {
    installMock({ scenario: 'trouble', speed: 0 })
    const r = await fetch('/api/v1/servers/kora-dev/control', { method: 'POST', headers: { 'X-Kora': '1' }, body: JSON.stringify({ action: 'start' }) })
    const { job } = (await r.json()) as { job: string }
    await waitFor(async () => expect(((await (await get(`/jobs/${job}`)).json()) as { state: string }).state).toBe('failed'))
    const j = (await (await get(`/jobs/${job}`)).json()) as { error: { code: string; message: string } }
    expect(j.error.message).toMatch(/already in use/)
    const t = await fetch('/api/v1/servers/kora-live/webhook/test', { method: 'POST', headers: { 'X-Kora': '1' } })
    expect(t.ok).toBe(false)
    expect(((await (await get('/servers/kora-live/logs')).json()) as { files: unknown[] }).files).toEqual([])
  })

  it('GitHub status tells "signed out" (default) and "gh missing" (trouble) apart, each with its fix', async () => {
    installMock({ scenario: 'default', speed: 0 })
    const out = (await (await get('/github/status')).json()) as GithubStatus
    expect([out.state, out.problem?.actions]).toEqual(['signed_out', [{ kind: 'sign_in_github' }]])
    installMock({ scenario: 'trouble', speed: 0 })
    const missing = (await (await get('/github/status')).json()) as GithubStatus
    expect([missing.state, missing.problem?.code, missing.problem?.actions?.[0]]).toEqual(['gh_missing', 'github.gh_missing', { kind: 'install_tool', tool: 'gh' }])
  })

  it('a stopped install can be continued once, and only its optional failed step skipped', async () => {
    installMock({ scenario: 'default', speed: 0 })
    const post = (path: string, body: unknown) => fetch(`/api/v1${path}`, { method: 'POST', headers: { 'X-Kora': '1' }, body: JSON.stringify(body) })
    const req = { mode: 'production', name: 'fail-town', dir: '/srv/fivem/fail-town', hostname: 'x', license_key: 'k', ports: { game: 30150, txadmin: 40150 }, txadmin_public: false, artifact_url: null, game_build: null, resources: 'all', db: 'use_existing', service: false, firewall: false, github_owner: null, dry_run: false, db_admin: null, start: true, cfx_identifier: null, app: null }
    const { job } = (await (await post('/installs', req)).json()) as { job: string }
    const ended = async (id: string) => {
      let j: Job = { state: 'running' } as Job
      await waitFor(async () => {
        j = (await (await get(`/jobs/${id}`)).json()) as Job
        expect(j.state).not.toBe('running')
      })
      return j
    }
    const failed = await ended(job)
    expect([failed.error?.code, failed.error?.failed_step, failed.error?.resume]).toEqual(['install.db_login_failed', 'build.db', true])
    expect((await post(`/installs/${job}/resume`, { skip: 'build.db' })).status).toBe(409)
    const again = (await (await post(`/installs/${job}/resume`, {})).json()) as { job: string }
    expect((await ended(again.job)).state).toBe('done')
    const twice = await post(`/installs/${job}/resume`, {})
    expect(twice.status).toBe(404)
    expect(((await twice.json()) as { error: { code: string } }).error.code).toBe('install.cannot_resume')
  })

  it('every scenario name is known to the switcher', () => {
    expect([...SCENARIOS]).toEqual(['default', 'empty', 'fleet', 'trouble', 'slow', 'offline', 'down'])
  })
})

describe('styleguide', () => {
  it('renders every section without errors', async () => {
    installMock({ scenario: 'default', speed: 0 })
    renderApp(<Styleguide />)
    for (const h of ['Brand colours', 'Typography', 'Buttons', 'Form controls', 'Badges and status chips', 'Alerts and error notices', 'Job progress', 'Cards and empty state', 'Table with long content', 'Console view', 'Dialogs, tabs, loading']) {
      expect(screen.getByRole('heading', { name: h })).toBeInTheDocument()
    }
    expect(screen.getByText('#95FF00')).toBeInTheDocument()
    expect(await screen.findAllByLabelText('Command to run')).not.toHaveLength(0)
  })
})
