import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { screen } from '@testing-library/react'
import type { JobSummary } from '@/api/types'
import { jsonResponse, renderApp } from '@/test/utils'
import { JobHistoryCard } from './JobHistory'

const fetchMock = vi.fn()
beforeEach(() => { fetchMock.mockReset(); vi.stubGlobal('fetch', fetchMock) })
afterEach(() => vi.unstubAllGlobals())

const job = (id: string, over: Partial<JobSummary> = {}): JobSummary => ({ id, op: 'update_apply', server: 'live', state: 'done', events: [], started: 1_700_000_000, ended: 1_700_000_050, result: {}, error: null, ...over })

describe('JobHistoryCard', () => {
  it('shows jobs from before a restart, with what to do for one that was interrupted', async () => {
    fetchMock.mockResolvedValue(jsonResponse(200, {
      jobs: [
        job('a', { op: 'install_start', server: 'Staging', state: 'interrupted', error: { code: 'job.interrupted', message: 'Kora Panel was closed while this install was running, so it did not finish.', resume: true } }),
        job('b', { state: 'failed', error: { code: 'failed', message: 'The update could not be downloaded.' } }),
        job('c'),
      ],
    }))
    renderApp(<JobHistoryCard />)
    expect(await screen.findByText('Stopped by a restart')).toBeInTheDocument()
    expect(screen.getByText(/Kora Panel was closed while this install was running/)).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Open Install to carry on' })).toHaveAttribute('href', '/install')
    expect(screen.getByText('The update could not be downloaded.')).toBeInTheDocument()
    expect(screen.getByText('Did not finish')).toBeInTheDocument()
    expect(screen.getByText('Finished')).toBeInTheDocument()
    expect(screen.getAllByTestId('job-row')).toHaveLength(3)
    expect(String(fetchMock.mock.calls[0][0])).toContain('/api/v1/jobs?limit=30')
  })

  it('says so when nothing has run', async () => {
    fetchMock.mockResolvedValue(jsonResponse(200, { jobs: [] }))
    renderApp(<JobHistoryCard />)
    expect(await screen.findByText('Nothing has run yet.')).toBeInTheDocument()
  })
})
