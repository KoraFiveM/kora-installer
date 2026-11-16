import { describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import { ApiError } from '@/api/client'
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
