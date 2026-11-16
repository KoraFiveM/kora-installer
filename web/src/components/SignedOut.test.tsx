import { describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { SignedOut } from './SignedOut'

describe('SignedOut on this computer', () => {
  it('does not offer Tailscale sign-in, which only works from other devices', () => {
    render(<SignedOut error={null} onToken={vi.fn()} onIdentity={vi.fn()} />)
    expect(screen.queryByRole('button', { name: /tailscale/i })).toBeNull()
    expect(screen.getByLabelText('Sign-in link')).toBeTruthy()
  })

  it('says how to get a new sign-in link and what to paste', () => {
    render(<SignedOut error={null} onToken={vi.fn()} />)
    expect(screen.getByRole('heading', { name: 'Sign in to Kora' })).toBeTruthy()
    expect(screen.getByText('kora panel --print-link')).toBeTruthy()
    expect(screen.getByText(/on this computer to get a new sign-in link\./)).toBeTruthy()
    expect(screen.getByLabelText('Sign-in link').getAttribute('aria-describedby')).toBeTruthy()
    expect(screen.getByText(/Paste the whole link the command printed/)).toBeTruthy()
    expect(screen.queryByLabelText('User token')).toBeNull()
  })

  it('passes a pasted link on as it is (sign-in reads the code out of it)', async () => {
    const onToken = vi.fn().mockResolvedValue(null)
    render(<SignedOut error={null} onToken={onToken} />)
    await userEvent.type(screen.getByLabelText('Sign-in link'), 'http://127.0.0.1:47120/#code=ab12')
    await userEvent.click(screen.getByRole('button', { name: 'Sign in' }))
    expect(onToken).toHaveBeenCalledWith('http://127.0.0.1:47120/#code=ab12')
  })
})
