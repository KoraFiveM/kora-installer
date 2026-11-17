import { describe, expect, it } from 'vitest'
import { screen } from '@testing-library/react'
import type { ResourceBranch } from '@/api/types'
import { renderApp } from '@/test/utils'
import { isOffNormal, NotNormalBadge, offNormalSummary, offNormalText } from './NotNormalBadge'

const row = (over: Partial<ResourceBranch>): ResourceBranch => ({ id: 'kora_hud', folder: 'kora_hud', state: 'default', default_branch: 'dev', label: '', changed_count: 0, stashes: 0, ...over })

describe('the not-on-its-normal-version badge', () => {
  it('draws nothing for the normal version and for resources with no branch', () => {
    const { container } = renderApp(<><NotNormalBadge branch={row({})} /><NotNormalBadge branch={row({ state: 'pinned' })} /><NotNormalBadge branch={undefined} /></>)
    expect(container.textContent).toBe('')
  })
  it('says it in words, not only colour', () => {
    renderApp(<NotNormalBadge branch={row({ state: 'pr', pr: 12 })} />)
    expect(screen.getByText('Trying pull request #12')).toBeInTheDocument()
    expect(screen.getByText(/Not on its normal version/)).toHaveClass('sr-only')
  })
  it('one wording for every kind', () => {
    expect(offNormalText(row({ state: 'branch', branch: 'fix' }))).toBe('On branch fix')
    expect(offNormalText(row({ state: 'detached' }))).toBe('On a single commit')
    expect(offNormalText(row({ state: 'pr', pr: 12, pr_title: 'Fix the HUD' }))).toBe('Trying pull request #12: Fix the HUD')
    expect(isOffNormal(row({ state: 'unavailable' }))).toBe(false)
  })
  it('summarises a server', () => {
    expect(offNormalSummary([row({}), row({ state: 'branch', branch: 'a' })])).toBe('1 resource not on its normal version')
    expect(offNormalSummary([row({ state: 'pr', pr: 1 }), row({ state: 'branch', branch: 'a' })])).toBe('2 resources not on their normal version')
    expect(offNormalSummary([row({})])).toBeNull()
  })
})
