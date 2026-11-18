import { beforeAll, describe, expect, it } from 'vitest'
import { screen } from '@testing-library/react'
import JoinHelpPage from './JoinHelpPage'
import { installMock } from '@/api/mock'
import { renderApp } from '@/test/utils'

beforeAll(() => installMock())

describe('Join help page', () => {
  it('walks through the steps in order and uses the default game port without a server', async () => {
    renderApp(<JoinHelpPage />, '/help/join')
    expect(await screen.findByRole('heading', { name: 'Players cannot join my server' })).toBeInTheDocument()
    const steps = screen.getAllByText(/^\d\. /).map((e) => e.textContent?.slice(0, 2))
    expect(steps).toEqual(['1.', '2.', '3.', '4.', '5.', '6.', '7.'])
    expect(screen.getByText(/The game port is 30120/)).toBeInTheDocument()
    expect(screen.getByText('connect 127.0.0.1:30120')).toBeInTheDocument()
    expect(screen.getByText('http://192.168.1.1')).toBeInTheDocument()
  })
})
