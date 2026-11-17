import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { Suspense, lazy } from 'react'
import { ErrorBoundary } from './ErrorBoundary'

afterEach(() => vi.restoreAllMocks())

const Broken = ({ error }: { error: Error }): never => {
  throw error
}

describe('ErrorBoundary', () => {
  it('shows a general problem with a way to try again and a way out', () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    render(<ErrorBoundary><Broken error={new Error('boom')} /></ErrorBoundary>)
    expect(screen.getByRole('heading', { name: 'Something went wrong' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Try again' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Back to your servers' })).toBeInTheDocument()
  })

  it('a page file that cannot be loaded (Kora was updated) offers Reload, which reloads the page', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    const reload = vi.fn()
    vi.stubGlobal('location', { ...window.location, reload })
    const Gone = lazy(() => Promise.reject(new TypeError('Failed to fetch dynamically imported module: /assets/Settings-old.js')))
    render(<ErrorBoundary><Suspense fallback={<p>loading</p>}><Gone /></Suspense></ErrorBoundary>)
    expect(await screen.findByRole('heading', { name: 'This page needs to be reloaded' })).toBeInTheDocument()
    expect(screen.getByText(/Kora was probably updated/)).toBeInTheDocument()
    // "Try again" cannot help here (the failed import is remembered), so it is not offered.
    expect(screen.queryByRole('button', { name: 'Try again' })).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: 'Reload the page' }))
    expect(reload).toHaveBeenCalledTimes(1)
    vi.unstubAllGlobals()
  })
})
