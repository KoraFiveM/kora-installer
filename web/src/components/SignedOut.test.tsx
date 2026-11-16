import { describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { SignedOut } from './SignedOut'

describe('SignedOut on this computer', () => {
  it('does not offer Tailscale sign-in, which only works from other devices', () => {
    render(<SignedOut error={null} onToken={vi.fn()} onIdentity={vi.fn()} />)
    expect(screen.queryByRole('button', { name: /tailscale/i })).toBeNull()
    expect(screen.getByLabelText('Sign-in code or link')).toBeTruthy()
  })

  it('passes a pasted link on as it is (sign-in reads the code out of it)', async () => {
    const onToken = vi.fn().mockResolvedValue(null)
    render(<SignedOut error={null} onToken={onToken} />)
    await userEvent.type(screen.getByLabelText('Sign-in code or link'), 'http://127.0.0.1:47120/#code=ab12')
    await userEvent.click(screen.getByRole('button', { name: 'Sign in' }))
    expect(onToken).toHaveBeenCalledWith('http://127.0.0.1:47120/#code=ab12')
  })
})
