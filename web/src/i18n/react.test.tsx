import { act, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import { register, setLocale } from './index'
import { rich, useLocale } from './react'
import '@/i18n/en/errors/errorNotice'
import '@/i18n/en/errors/toolchainActions'

afterEach(async () => {
  await setLocale('en', false)
})

function Where() {
  return <span>{useLocale()}</span>
}

describe('rich()', () => {
  it('puts the tags and pieces where the message says, with no HTML parsing', () => {
    // "{link, node} to see what Kora needs on this computer."
    render(<p>{rich('errors.errorNotice.linkSeeWhatKora', { link: <a href="/system">Open the System check</a> })}</p>)
    const a = screen.getByRole('link', { name: 'Open the System check' })
    expect(a.parentElement?.textContent).toBe('Open the System check to see what Kora needs on this computer.')
  })

  it('a translation can put a tag anywhere, and text in a param is never read as markup', async () => {
    register('de', { 'errors.errorNotice.linkSeeWhatKora': 'Was Kora auf diesem Computer braucht, siehst du hier: {link, node}' })
    await setLocale('de', false)
    render(<p>{rich('errors.errorNotice.linkSeeWhatKora', { link: <b>{'<img src=x onerror=alert(1)>'}</b> })}</p>)
    expect(screen.getByText('<img src=x onerror=alert(1)>')).toBeInTheDocument()
    expect(document.querySelector('img')).toBeNull()
  })

  it('useLocale re-renders when the language changes', async () => {
    render(<Where />)
    expect(screen.getByText('en')).toBeInTheDocument()
    await act(async () => {
      await setLocale('de', false)
    })
    expect(screen.getByText('de')).toBeInTheDocument()
  })
})
