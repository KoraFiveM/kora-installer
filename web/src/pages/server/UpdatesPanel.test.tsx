import { describe, expect, it, vi } from 'vitest'
import { render, screen, within } from '@testing-library/react'
import { ChangeRow } from './UpdatesPanel'
import { CountBar } from '@/components/JobProgress'
import type { ResourceChange } from '@/api/types'

const row = (id: string): ResourceChange => ({ id, from: 'aaaaaaa', to: 'bbbbbbb', behind: 1, changelog: [], needs_restart: false, status: 'clean' })

describe('CountBar', () => {
  it('shows the count and the bar at the right value', () => {
    render(<CountBar label="Updating resources" done={2} total={4} text="2 of 4 updated" detail="Updating kora_hud" />)
    const bar = screen.getByRole('progressbar', { name: 'Updating resources' })
    expect(bar).toHaveAttribute('aria-valuenow', '50')
    expect(screen.getByText('2 of 4 updated')).toBeInTheDocument()
    expect(screen.getByText('Updating kora_hud')).toBeInTheDocument()
  })

  it('is 100% when everything is done, and 0% with nothing to count', () => {
    const { rerender } = render(<CountBar label="Checking" done={3} total={3} text="3 of 3" />)
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '100')
    rerender(<CountBar label="Checking" done={0} total={0} text="Starting..." />)
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '0')
  })
})

describe('ChangeRow per-resource check bar', () => {
  const base = { selected: false, onToggle: vi.fn(), onCheck: vi.fn(), disabled: false, now: 0 }

  it('shows the inline bar only on the row that is being checked', () => {
    const progress = { done: 1, total: 1, id: 'kora_hud' }
    render(
      <ul>
        <ChangeRow {...base} r={row('kora_core')} checking={false} />
        <ChangeRow {...base} r={row('kora_hud')} checking progress={progress} />
      </ul>,
    )
    const items = screen.getAllByRole('listitem')
    expect(within(items[0]).queryByRole('progressbar')).toBeNull()
    const bar = within(items[1]).getByRole('progressbar', { name: 'Checking kora_hud' })
    expect(bar).toHaveAttribute('aria-valuenow', '100')
    expect(within(items[1]).getByText('Checked kora_hud')).toBeInTheDocument()
  })

  it('a row with no progress has no bar at all', () => {
    render(<ul><ChangeRow {...base} r={row('kora_core')} checking={false} /></ul>)
    expect(screen.queryByRole('progressbar')).toBeNull()
  })
})
