import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { ImportPreview, Meta } from '@/api/types'
import { jsonResponse, renderApp } from '@/test/utils'
import ImportPage from './ImportPage'

const job = vi.hoisted(() => ({ view: { phase: 'idle', events: [] as unknown[], result: null as unknown, error: null as unknown } }))
vi.mock('@/hooks/useJob', () => ({ useJob: (id: string | null) => (id ? job.view : { phase: 'idle', events: [], result: null, error: null }) }))

const fetchMock = vi.fn()
beforeEach(() => { fetchMock.mockReset(); job.view = { phase: 'idle', events: [], result: null, error: null }; vi.stubGlobal('fetch', fetchMock) })
afterEach(() => vi.unstubAllGlobals())

const preview = (o: Partial<ImportPreview> = {}): ImportPreview => ({
  file_name: 'old.tar.gz.age',
  encrypted: true,
  needs_passphrase: false,
  created_unix: 1,
  server: { name: 'Old RP', mode: 'production', game_port: 30120, txadmin_port: 40120, game_build: 3258, artifact_build: 17000, os: 'windows', dir: 'C:\\Servers\\old' },
  database: { kind: 'dumped', name: 'kora_x', host: '127.0.0.1', port: 3306, dump_bytes: 100, note: '' },
  by_reference: 38,
  in_full: 2,
  suggested: { name: 'Old RP', dir: '/home/me/kora/old-rp', game_port: 30121, txadmin_port: 40121 },
  clashes: [{ kind: 'game_port', message: 'The game port 30120 is already used on this computer.', suggestion: '30121' }],
  will_create: ['A new server "Old RP" in /home/me/kora/old-rp'],
  wont_carry: ['The operating-system service that starts the server with the computer.'],
  warnings: ['This server was made on a different operating system.'],
  saved_keys: ['Main', 'Spare'],
  licence_note: "Your licence key moves with the server, but it is yours, not Kora's: Cfx.re ties a key to your account. Kora cannot make Cfx.re accept it on a new computer.",
  ...o,
})

function renderIt(meta?: Partial<Meta>) {
  const r = renderApp(<ImportPage />, '/import')
  void meta
  return r
}

describe('ImportPage', () => {
  it('asks for a file, then the password, then shows what will happen, with the licence warning and what does not come along', async () => {
    const user = userEvent.setup()
    fetchMock.mockImplementation(async (url: string, init?: RequestInit) => {
      if (String(url).endsWith('/backups/import/inspect')) {
        const body = JSON.parse(String(init?.body))
        return body.passphrase === 'right one' ? jsonResponse(200, preview()) : jsonResponse(200, preview({ needs_passphrase: true, server: null, database: null, suggested: null, clashes: [], will_create: [], wont_carry: [], warnings: [] }))
      }
      return jsonResponse(200, {})
    })
    renderIt()
    await user.type(screen.getByLabelText(/Or give the file's path/), '/mnt/usb/old.tar.gz.age')
    await user.click(screen.getByRole('button', { name: 'Check this file' }))
    await user.type(await screen.findByLabelText('Its password'), 'right one')
    await user.click(screen.getByRole('button', { name: 'Open the file' }))
    expect(await screen.findByText('Kora will create:')).toBeInTheDocument()
    expect(screen.getByText(/Cfx.re ties a key to your account/)).toBeInTheDocument()
    expect(screen.getByText(/The operating-system service that starts the server/)).toBeInTheDocument()
    expect(screen.getByText('This server was made on a different operating system.')).toBeInTheDocument()
    expect(screen.getByText('made on Windows')).toBeInTheDocument()
  })

  it('offers the fix for a clash, lets a saved key be chosen, and starts the import with exactly those choices', async () => {
    const user = userEvent.setup()
    fetchMock.mockImplementation(async (url: string) => (String(url).endsWith('/backups/import') ? jsonResponse(202, { job: 'job-1' }) : jsonResponse(200, preview({ needs_passphrase: false, suggested: { name: 'Old RP', dir: '/home/me/kora/old-rp', game_port: 30120, txadmin_port: 40121 } }))))
    renderIt()
    await user.type(screen.getByLabelText(/Or give the file's path/), '/x/old.tar.gz.age')
    await user.click(screen.getByRole('button', { name: 'Check this file' }))
    await screen.findByText('Kora will create:')
    await user.click(screen.getByRole('button', { name: 'Use 30121' }))
    expect(screen.getByLabelText('Game port')).toHaveValue(30121)
    await user.click(screen.getByLabelText('Use one of my saved keys instead'))
    await user.selectOptions(screen.getByLabelText('Saved licence key'), 'Spare')
    await user.click(screen.getByLabelText('Start the server when it is ready'))
    await user.click(screen.getByRole('button', { name: 'Bring the server in' }))
    await waitFor(() => expect(fetchMock.mock.calls.some(([u]) => String(u).endsWith('/backups/import'))).toBe(true))
    const call = fetchMock.mock.calls.find(([u]) => String(u).endsWith('/backups/import'))!
    expect(JSON.parse(String(call[1].body))).toEqual({
      source: { path: '/x/old.tar.gz.age' },
      choices: { name: 'Old RP', dir: '/home/me/kora/old-rp', game_port: 30121, txadmin_port: 40121 },
      licence_key_name: 'Spare',
      start: false,
      start_over: false,
    })
  })

  it('shows the result with the instructions for the old computer, and a failure offers carry on or start over', async () => {
    const user = userEvent.setup()
    fetchMock.mockImplementation(async (url: string) => (String(url).endsWith('/backups/import') ? jsonResponse(202, { job: 'job-2' }) : jsonResponse(200, preview())))
    job.view = { phase: 'done', events: [], error: null, result: { name: 'Old RP', dir: '/home/me/kora/old-rp', resources_fetched: [], problems: ['kora_hud: could not download'], notes: [], started: false, next_steps: ['Open the ports'], old_machine_steps: ['Remove it from the OLD computer'] } }
    renderIt()
    await user.type(screen.getByLabelText(/Or give the file's path/), '/x/old.tar.gz.age')
    await user.click(screen.getByRole('button', { name: 'Check this file' }))
    await user.click(await screen.findByRole('button', { name: 'Bring the server in' }))
    expect(await screen.findByText('Old RP is imported')).toBeInTheDocument()
    expect(screen.getByText('What did not work')).toBeInTheDocument()
    expect(screen.getByText('Remove it from the OLD computer')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Open Old RP' })).toHaveAttribute('href', '/servers/Old%20RP')
  })

  it('a failed import offers to carry on or start over', async () => {
    const user = userEvent.setup()
    fetchMock.mockImplementation(async (url: string) => (String(url).endsWith('/backups/import') ? jsonResponse(202, { job: 'job-3' }) : jsonResponse(200, preview())))
    job.view = { phase: 'failed', events: [], result: null, error: { message: 'Kora could not create the new database. Nothing is registered yet.', code: 'backup.db_stopped', actions: [{ kind: 'start_database' }] } as never }
    renderIt()
    await user.type(screen.getByLabelText(/Or give the file's path/), '/x/old.tar.gz.age')
    await user.click(screen.getByRole('button', { name: 'Check this file' }))
    await user.click(await screen.findByRole('button', { name: 'Bring the server in' }))
    expect(await screen.findByText(/Nothing is registered yet/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: /Start over/ }))
    await waitFor(() => expect(fetchMock.mock.calls.filter(([u]) => String(u).endsWith('/backups/import')).length).toBe(2))
    const last = fetchMock.mock.calls.filter(([u]) => String(u).endsWith('/backups/import')).at(-1)!
    expect(JSON.parse(String(last[1].body)).start_over).toBe(true)
  })
})
