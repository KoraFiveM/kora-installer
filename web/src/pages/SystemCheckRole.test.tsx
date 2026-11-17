import { describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router-dom'
import SystemCheckPage from './SystemCheckPage'
import { ToolchainActions } from '@/components/ToolchainActions'
import { installMock } from '@/api/mock'

function renderAs(role: 'admin' | 'operator') {
  installMock({ scenario: 'trouble', speed: 0 })
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  qc.setQueryData(['meta'], { version: '0', api: 1, platform: 'linux', ops: [], node: 'n', remote: true, identity: { name: 'olga', provider: 'token', role } })
  return render(<QueryClientProvider client={qc}><MemoryRouter><ToolchainActions><SystemCheckPage /></ToolchainActions></MemoryRouter></QueryClientProvider>)
}

describe('System check for different roles', () => {
  it('gives an operator no fix buttons, and says who can fix it', async () => {
    renderAs('operator')
    expect(await screen.findByText(/can look at this but not fix it/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /for me|Sign in to GitHub|Start it for me/ })).not.toBeInTheDocument()
  })

  it('gives an administrator the fix buttons', async () => {
    renderAs('admin')
    expect(await screen.findByRole('button', { name: 'Install the GitHub CLI for me' })).toBeInTheDocument()
    expect(screen.queryByText(/can look at this but not fix it/)).not.toBeInTheDocument()
  })
})
