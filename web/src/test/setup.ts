import '@testing-library/jest-dom/vitest'
import { cleanup } from '@testing-library/react'
import { afterEach } from 'vitest'
import { forgetJobs } from '@/hooks/useJob'
import { clearKept } from '@/lib/kept'

afterEach(() => {
  cleanup()
  // The session secret lives in localStorage: no test may leak it into the next.
  window.localStorage.clear()
  // Kept page state and followed jobs are per page load: every test starts with none.
  clearKept()
  forgetJobs()
})
