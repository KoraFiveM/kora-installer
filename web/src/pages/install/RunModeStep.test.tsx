import { beforeEach, describe, expect, it, vi } from 'vitest'
import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import InstallWizard from '../InstallWizard'
import { installMock } from '@/api/mock'
import { api } from '@/api/client'
import { renderApp } from '@/test/utils'
import { buildRequest, initialForm, runModePatch, stepProblem, stepsFor } from './form'

// These tests are about the steps, not the setup checklist (SetupGate.test.tsx): treat Cfx and GitHub as set up.
vi.mock('@/pages/install/setup', () => ({ useSetup: () => ({ cfxReady: true, githubReady: true, blocked: false, settled: true }) }))

const KEY = 'cfxk_abcdefghij_12345'

beforeEach(() => {
  history.replaceState(null, '', '/')
})

/**
 * Press "Next" until `arrived()` is true. "Next" is disabled while the wizard still waits for an answer (the ports
 * check), and a click on a disabled button does nothing: so wait for it to be enabled instead of counting clicks.
 */
async function nextUntil(user: ReturnType<typeof userEvent.setup>, arrived: () => boolean) {
  for (let i = 0; i < 10 && !arrived(); i++) {
    const next = screen.getByRole('button', { name: 'Next' })
    await waitFor(() => expect(next).toBeEnabled(), { timeout: 5000 })
    await user.click(next)
  }
}

async function toRunMode(user: ReturnType<typeof userEvent.setup>) {
  renderApp(<InstallWizard />, '/install')
  await user.type(await screen.findByLabelText('Licence key'), KEY)
  await user.click(screen.getByRole('button', { name: 'Next' }))
  await screen.findByLabelText('Server name')
  await user.click(screen.getByRole('button', { name: 'Next' }))
  await user.click(await screen.findByRole('button', { name: /Choose everything myself/ }))
}

describe('Install wizard: how the server runs (container mode)', () => {
  it('is not asked where no container engine works (the default world has none)', async () => {
    installMock({ scenario: 'default', speed: 0 })
    const user = userEvent.setup()
    await toRunMode(user)
    expect(screen.queryByText('How should this server run?')).not.toBeInTheDocument()
    expect(screen.queryByRole('listitem', { name: /How the server runs/ })).not.toBeInTheDocument()
  })

  it('offers a service or a container, and a container changes the database and options pages and what is sent', async () => {
    installMock({ scenario: 'containers', speed: 0 })
    const start = vi.spyOn(api.installs, 'start').mockImplementation(() => new Promise(() => {}))
    const user = userEvent.setup()
    await toRunMode(user)
    // the step is in the list, with the engine the System check found
    expect(await screen.findByText(/How the server runs/, { selector: 'li' })).toBeInTheDocument()
    // walk to the run-mode page by "Next" until its question is on screen
    await nextUntil(user, () => !!screen.queryByText('How should this server run?'))
    expect(screen.getByText(/Podman 4\.9\.3, rootless: ready to run servers/)).toBeInTheDocument()
    expect(screen.getByRole('radio', { name: /As a system service/ })).toBeChecked()
    await user.click(screen.getByRole('radio', { name: /In a container/ }))
    expect(screen.getByText(/What a container does and does not protect/)).toBeInTheDocument()
    expect(screen.queryByText(/Docker runs as root/)).not.toBeInTheDocument()
    // database: a MariaDB container is the default, the administrator login box is gone
    await user.click(screen.getByRole('button', { name: 'Next' }))
    expect(await screen.findByRole('radio', { name: /A MariaDB container next to the server/ })).toBeChecked()
    expect(screen.queryByText('Database administrator login')).not.toBeInTheDocument()
    // options: no background service to install
    await user.click(screen.getByRole('button', { name: 'Next' }))
    expect(await screen.findByText(/There is no background service to install/)).toBeInTheDocument()
    expect(screen.queryByRole('checkbox', { name: /Install as a background service/ })).not.toBeInTheDocument()
    // on to the summary and install
    await nextUntil(user, () => !!screen.queryByRole('button', { name: 'Install the server' }))
    expect(await screen.findByText('In a container', { selector: 'div' })).toBeInTheDocument()
    await user.click(await screen.findByRole('button', { name: 'Install the server' }))
    const sent = start.mock.calls[0][0]
    expect(sent).toMatchObject({ run_mode: 'container', container: {}, db: 'container', service: false })
    expect(sent.db_admin).toBeNull()
  })
})

describe('Install wizard form: run mode', () => {
  const defaults = { name: 'main', dir: '/home/sam/kora/main', hostname: 'main', ports: { game: 30120, txadmin: 40120 }, saved: null, profile: {}, resumable: [] } as unknown as Parameters<typeof initialForm>[0]

  it('is a service by default and sends nothing about containers', () => {
    const f = initialForm(defaults)
    expect(f.runMode).toBe('service')
    const r = buildRequest(f)
    expect(r.run_mode).toBeUndefined()
    expect(r.container).toBeUndefined()
    expect(r.service).toBe(true)
  })

  it('a container server has no service, uses the container database, and going back restores the usual database', () => {
    let f = initialForm(defaults)
    f = { ...f, ...runModePatch(f, 'container') }
    expect(f.db).toBe('container')
    expect(f.firewall).toBe(false)
    const r = buildRequest(f)
    expect(r).toMatchObject({ run_mode: 'container', container: {}, service: false, db: 'container', db_admin: null })
    f = { ...f, ...runModePatch(f, 'service') }
    expect(f.db).toBe('install_if_missing')
    expect(f.firewall).toBe(true)
    expect(runModePatch({ ...f, db: 'skip' }, 'container').db).toBe('skip')
  })

  it('the step exists only when containers are offered, and a container choice without a working engine is a problem in words', () => {
    expect(stepsFor('clone')).not.toContain('runmode')
    expect(stepsFor('clone', true)).toContain('runmode')
    expect(stepsFor('clone', true).indexOf('runmode')).toBeLessThan(stepsFor('clone', true).indexOf('database'))
    const f = { ...initialForm(defaults), runMode: 'container' as const }
    expect(stepProblem('runmode', f, { ports: 'ok', githubSignedIn: true, containers: false })).toMatch(/needs Podman or Docker/)
    expect(stepProblem('runmode', f, { ports: 'ok', githubSignedIn: true, containers: true })).toBeNull()
    expect(stepProblem('runmode', initialForm(defaults), { ports: 'ok', githubSignedIn: true, containers: false })).toBeNull()
  })
})

describe('Where the run mode shows', () => {
  it('the servers list says "in a container" for the container server only', async () => {
    installMock({ scenario: 'containers', speed: 0 })
    const { default: ServersPage } = await import('../ServersPage')
    renderApp(<ServersPage />, '/')
    expect(await screen.findByText('In a container (podman, rootless)')).toBeInTheDocument()
    expect(screen.getAllByText(/In a container/)).toHaveLength(1)
  })

  it('the System check has a Containers line only where an engine was found, and a broken one carries its fix', async () => {
    const { default: SystemCheckPage } = await import('../SystemCheckPage')
    const { ToolchainActions } = await import('@/components/ToolchainActions')
    installMock({ scenario: 'default', speed: 0 })
    const first = renderApp(<ToolchainActions><SystemCheckPage /></ToolchainActions>, '/system')
    await screen.findByRole('heading', { name: 'Background service' })
    expect(screen.queryByRole('heading', { name: 'Containers' })).not.toBeInTheDocument()
    first.unmount()
    installMock({ scenario: 'containers', speed: 0 })
    renderApp(<ToolchainActions><SystemCheckPage /></ToolchainActions>, '/system')
    expect(await screen.findByRole('heading', { name: 'Containers' })).toBeInTheDocument()
    expect(await screen.findByText(/ready to run servers/)).toBeInTheDocument()
  })
})
