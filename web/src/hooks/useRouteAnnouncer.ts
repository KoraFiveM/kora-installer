import { useEffect, useRef } from 'react'

/** The suffix of every page title: "Servers · Kora". */
export const TITLE_SUFFIX = 'Kora'

/**
 * Route changes in a single-page app are silent for a screen reader and leave the keyboard focus on a link that may no
 * longer exist. After each navigation this
 *  - waits for the page's `<h1>` to be in `<main>` (pages are lazy, so it can take a moment),
 *  - sets `document.title` to "<h1> · Kora" (and keeps it in step if the heading changes, for example when the server's
 *    name has loaded),
 *  - moves focus to that heading (a screen reader reads it out: that is the announcement), unless the page already put
 *    focus on something inside `<main>` (an auto-focused field).
 * The first load of the page only sets the title: focus stays where the browser puts it.
 * Returns the ref for `<main>` (also the target of the skip link).
 */
export function useRouteAnnouncer(pathname: string) {
  const main = useRef<HTMLElement>(null)
  const first = useRef(true)
  useEffect(() => {
    const isFirst = first.current
    first.current = false
    const root = main.current
    if (!root) return
    let focused = isFirst
    const apply = () => {
      const h1 = root.querySelector('h1')
      if (!h1) return false
      const text = h1.textContent?.trim()
      if (text) document.title = `${text} · ${TITLE_SUFFIX}`
      if (!focused) {
        focused = true
        const a = document.activeElement
        if (!(a instanceof HTMLElement) || a === document.body || !root.contains(a) || a === root) {
          h1.setAttribute('tabindex', '-1')
          h1.classList.add('outline-none')
          h1.focus({ preventScroll: true })
          window.scrollTo(0, 0)
        }
      }
      return true
    }
    apply()
    const obs = new MutationObserver(() => void apply())
    obs.observe(root, { childList: true, subtree: true, characterData: true })
    // A page with no heading yet gets a neutral title rather than the previous page's.
    if (!root.querySelector('h1')) document.title = TITLE_SUFFIX
    return () => {
      obs.disconnect()
    }
  }, [pathname])
  return main
}

/** For screens outside the app shell (sign-in, enrolment): the title of the page. */
export function usePageTitle(title: string) {
  useEffect(() => {
    document.title = `${title} · ${TITLE_SUFFIX}`
  }, [title])
}
