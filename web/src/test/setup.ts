import '@testing-library/jest-dom/vitest'
import { cleanup, configure } from '@testing-library/react'
import { afterEach } from 'vitest'
import { forgetJobs } from '@/hooks/useJob'
import { clearKept } from '@/lib/kept'

// Some screens answer only after a real delay (the wizard's ports check waits 250 ms before it asks) and a lazy
// chunk has to load first: `findBy*`/`waitFor` allow 1 s by default, which a shared CI runner can use up on
// import and render alone. A wait that succeeds still returns at once; only a failing one takes longer.
configure({ asyncUtilTimeout: 4000 })

afterEach(() => {
  cleanup()
  // The session secret lives in localStorage: no test may leak it into the next.
  window.localStorage.clear()
  // Kept page state and followed jobs are per page load: every test starts with none.
  clearKept()
  forgetJobs()
})
