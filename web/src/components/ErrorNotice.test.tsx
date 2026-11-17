import { describe, expect, it, vi } from 'vitest'
import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { ApiError, type ApiErrorBody } from '@/api/client'
import type { ProblemAction } from '@/api/types'
import { detailsText, ProblemActions, toProblem } from '@/lib/problemActions'
import { ErrorNotice } from './ErrorNotice'

describe('ErrorNotice', () => {
  it('shows an ordinary error as it is', () => {
    render(<ErrorNotice error={new ApiError(409, { code: 'conflict', message: 'Stop the server first, then delete it.' })} />)
    expect(screen.getByRole('alert')).toHaveTextContent('Stop the server first, then delete it.')
    expect(screen.queryByText(/Wait for it to finish/)).toBeNull()
    expect(screen.queryByRole('button', { name: 'Copy' })).toBeNull()
  })

  it('says what to do when another update is running', () => {
    render(<ErrorNotice error={new ApiError(409, { code: 'conflict', message: 'Another update is running for this server.' })} />)
    expect(screen.getByText('Another update is running for this server.')).toBeInTheDocument()
    expect(screen.getByText("Wait for it to finish, then try again. Its progress is on the server's Resources tab.")).toBeInTheDocument()
  })

  it('opens the System check in a new tab from the install wizard, so the typed answers are not lost', () => {
    window.history.pushState({}, '', '/install')
    render(<ErrorNotice error={new ApiError(500, { code: 'github.gh_missing', message: 'The GitHub CLI (gh) is not installed.' })} />)
    const link = screen.getByRole('link', { name: 'Open the System check in a new tab' })
    expect(link).toHaveAttribute('target', '_blank')
    window.history.pushState({}, '', '/')
  })

  it('offers the command of a remote sign-in refusal to copy', () => {
    const message = 'This action is turned off for remote sign-ins. To turn it on, run this on the computer that runs Kora: `kora panel remote-actions on`'
    render(<ErrorNotice error={new ApiError(403, { code: 'forbidden', message })} />)
    expect(screen.getByLabelText('Command to run')).toHaveTextContent('kora panel remote-actions on')
    expect(screen.getByRole('button', { name: 'Copy' })).toBeInTheDocument()
    expect(screen.getByRole('alert')).not.toHaveTextContent('`')
  })

  it('shows a role refusal without a command', () => {
    render(<ErrorNotice error={new ApiError(403, { code: 'forbidden', message: 'Your account has viewer access, so it cannot do this. Ask an administrator for operator access.' })} />)
    expect(screen.getByRole('alert')).toHaveTextContent('Ask an administrator for operator access.')
    expect(screen.queryByLabelText('Command to run')).toBeNull()
  })

  it('explains a request that never got an answer', () => {
    render(<ErrorNotice error={new ApiError(0, { code: 'network', message: 'Could not reach Kora. Check that it is still running, then try again.' })} />)
    const alert = screen.getByRole('alert')
    expect(alert).toHaveTextContent('Kora Panel did not answer. Check that it is still running on the server (kora panel restart restarts it), then reload this page.')
    expect(alert).not.toHaveTextContent('Could not reach Kora')
  })
})

describe('ErrorNotice with a Problem', () => {
  const problem = (actions: ProblemAction[], extra: Partial<ApiErrorBody> = {}) =>
    new ApiError(422, { code: 'install.step_failed', message: 'The download did not finish.', details: 'HTTP 503 from runtime.fivem.net', actions, ...extra })

  it('shows the message, folds the details away, and draws one button per action it can do', async () => {
    const user = userEvent.setup()
    const retry = vi.fn()
    const skip = vi.fn()
    render(<ErrorNotice error={problem([{ kind: 'retry_step', step: 'build.artifact' }, { kind: 'skip_step', step: 'build.artifact' }, { kind: 'install_tool', tool: 'gh' }, { kind: 'change_answers', section: 'ports' }])} actions={{ retry_step: retry, skip_step: skip }} />)
    const alert = screen.getByRole('alert')
    expect(alert).toHaveTextContent('The download did not finish.')
    const details = screen.getByText('HTTP 503 from runtime.fivem.net')
    expect(details.closest('details')).not.toHaveAttribute('open')
    // Only actions with a handler are buttons ("install the GitHub CLI" and "change answers" have none here).
    expect(within(alert).getAllByRole('button').map((b) => b.textContent)).toEqual(['Try this step again', 'Skip this step'])
    await user.click(screen.getByRole('button', { name: 'Skip this step' }))
    expect(skip).toHaveBeenCalledWith({ kind: 'skip_step', step: 'build.artifact' }, expect.anything())
    expect(retry).not.toHaveBeenCalled()
  })

  it('takes handlers from the app-wide registry, and a page handler wins', async () => {
    const user = userEvent.setup()
    const appWide = vi.fn()
    const page = vi.fn()
    render(
      <ProblemActions handlers={{ install_tool: appWide, change_answers: appWide }}>
        <ErrorNotice error={problem([{ kind: 'install_tool', tool: 'gh' }, { kind: 'change_answers', section: 'ports' }])} actions={{ change_answers: page }} />
      </ProblemActions>,
    )
    await user.click(screen.getByRole('button', { name: 'Install the GitHub CLI for me' }))
    expect(appWide).toHaveBeenCalledWith({ kind: 'install_tool', tool: 'gh' }, expect.anything())
    await user.click(screen.getByRole('button', { name: 'Change the ports' }))
    expect(page).toHaveBeenCalledTimes(1)
    expect(appWide).toHaveBeenCalledTimes(1)
  })

  it('hands the handler what to do once a fix worked: onFixed, or else onRetry', async () => {
    const user = userEvent.setup()
    const fix = vi.fn()
    const fixed = vi.fn()
    const retry = vi.fn()
    const { rerender } = render(<ErrorNotice error={problem([{ kind: 'install_tool', tool: 'git' }])} actions={{ install_tool: fix }} onRetry={retry} />)
    await user.click(screen.getByRole('button', { name: 'Install Git for me' }))
    expect(fix.mock.calls[0][1].afterFix).toBe(retry)
    rerender(<ErrorNotice error={problem([{ kind: 'install_tool', tool: 'git' }])} actions={{ install_tool: fix }} onRetry={retry} onFixed={fixed} />)
    await user.click(screen.getByRole('button', { name: 'Install Git for me' }))
    expect(fix.mock.calls[1][1].afterFix).toBe(fixed)
    rerender(<ErrorNotice error={problem([{ kind: 'install_tool', tool: 'git' }])} actions={{ install_tool: fix }} />)
    await user.click(screen.getByRole('button', { name: 'Install Git for me' }))
    expect(fix.mock.calls[2][1].afterFix).toBeUndefined()
  })

  it('draws commands, https links and the log path without any handler', () => {
    render(<ErrorNotice error={problem([{ kind: 'show_command', command: ['gh', 'auth', 'login'] }, { kind: 'open_link', url: 'https://cli.github.com' }, { kind: 'open_link', url: 'javascript:alert(1)' }, { kind: 'open_log', path: '/srv/x/.kora/install.log' }])} />)
    expect(screen.getByLabelText('Command to run')).toHaveTextContent('gh auth login')
    const links = screen.getAllByRole('link')
    expect(links).toHaveLength(1)
    expect(links[0]).toHaveAttribute('href', 'https://cli.github.com')
    expect(links[0]).toHaveTextContent('Open cli.github.com')
    expect(screen.getByText('/srv/x/.kora/install.log')).toBeInTheDocument()
  })

  it('falls back to "Try again" and "Copy details" for a plain failure', async () => {
    const user = userEvent.setup()
    const again = vi.fn()
    render(<ErrorNotice error={{ code: 'failed', message: 'disk full' }} onRetry={again} />)
    await user.click(screen.getByRole('button', { name: 'Try again' }))
    expect(again).toHaveBeenCalled()
    expect(screen.getByRole('button', { name: 'Copy details' })).toBeInTheDocument()
  })

  it('ignores actions it does not know and keeps the old elevation command', () => {
    const e = new ApiError(403, { code: 'needs_elevation', message: 'This install needs administrator rights.', command: ['kora', 'install', '--yes'], actions: [{ kind: 'teleport' } as unknown as ProblemAction] })
    render(<QueryClientProvider client={new QueryClient()}><ErrorNotice error={e} /></QueryClientProvider>)
    expect(screen.getByRole('alert')).toHaveTextContent('This needs administrator rights')
    expect(screen.getByLabelText('Command to run')).toHaveTextContent('kora install --yes')
    expect(toProblem(e)?.actions).toEqual([])
  })

  it('copies the message, the details and the code', () => {
    expect(detailsText({ code: 'github.gh_missing', message: 'No gh.', details: 'which gh: not found', actions: [] })).toBe('No gh.\n\nwhich gh: not found\n\n(code: github.gh_missing)')
  })
})
