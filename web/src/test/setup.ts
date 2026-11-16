import '@testing-library/jest-dom/vitest'
import { cleanup } from '@testing-library/react'
import { afterEach } from 'vitest'

afterEach(() => {
  cleanup()
  // The session secret lives in localStorage: no test may leak it into the next.
  window.localStorage.clear()
})
