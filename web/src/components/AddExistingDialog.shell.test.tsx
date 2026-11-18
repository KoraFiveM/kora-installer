import { afterEach, describe, expect, it, vi } from 'vitest'
import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { AddExistingDialog } from '@/components/AddExistingDialog'
import { renderApp } from '@/test/utils'
import type { KoraShell } from '@/lib/shell'

afterEach(() => {
  delete window.koraShell
  vi.unstubAllGlobals()
})

function shell(path: string | null): KoraShell {
  return {
    hello: vi.fn(),
    pickFile: vi.fn(),
    pickFolder: vi.fn(async () => path),
    openExternal: vi.fn(),
    notify: vi.fn(),
    elevate: vi.fn(),
  } as unknown as KoraShell
}

describe('Add an existing server, inside the desktop app', () => {
  it('Browse fills the folder box from the system dialog', async () => {
    window.koraShell = shell('C:\\FiveM\\my-server')
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ role: 'admin' }), { status: 200 })))
    renderApp(<AddExistingDialog open onOpenChange={() => undefined} />)
    await userEvent.click(await screen.findByRole('button', { name: 'Browse...' }))
    await waitFor(() => expect(screen.getByLabelText('Server folder')).toHaveValue('C:\\FiveM\\my-server'))
  })

  it('a cancelled dialog leaves the box as it was', async () => {
    window.koraShell = shell(null)
    renderApp(<AddExistingDialog open onOpenChange={() => undefined} />)
    await userEvent.type(screen.getByLabelText('Server folder'), '/srv/x')
    await userEvent.click(screen.getByRole('button', { name: 'Browse...' }))
    expect(screen.getByLabelText('Server folder')).toHaveValue('/srv/x')
  })

  it('in a plain browser there is no Browse button', () => {
    renderApp(<AddExistingDialog open onOpenChange={() => undefined} />)
    expect(screen.queryByRole('button', { name: 'Browse...' })).toBeNull()
  })
})
