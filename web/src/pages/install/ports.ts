import { useCallback, useEffect, useState } from 'react'
import { api } from '@/api/client'
import type { PortsCheck } from '@/api/types'
import { validatePorts } from '@/lib/utils'
import type { PortsState } from './form'

export interface PortsChecked {
  /** The answer for the ports as typed now (null while asking, or when the numbers are not valid). */
  check: PortsCheck | null
  /** Set when the check itself could not run. */
  error: unknown
  state: PortsState
  retry: () => void
}

/**
 * Asks Kora whether the two ports are free, a moment after they change. It lives in the wizard, not
 * in the Ports step, so the suggested ports are checked even when nobody opens that step.
 */
export function usePortsCheck(game: string, tx: string): PortsChecked {
  const [check, setCheck] = useState<PortsCheck | null>(null)
  const [error, setError] = useState<unknown>(null)
  const [state, setState] = useState<PortsState>('checking')
  const [attempt, setAttempt] = useState(0)
  const invalid = validatePorts(game, tx)
  useEffect(() => {
    setCheck(null)
    setError(null)
    setState('checking')
    if (invalid) return
    let stale = false
    const t = setTimeout(() => {
      api.installs.checkPorts({ game: Number(game), txadmin: Number(tx) }).then(
        (c) => { if (!stale) { setCheck(c); setState(c.game.ok && c.txadmin.ok ? 'ok' : 'taken') } },
        (e) => { if (!stale) { setError(e); setState('unknown') } },
      )
    }, 250)
    return () => { stale = true; clearTimeout(t) }
  }, [game, tx, invalid, attempt])
  const retry = useCallback(() => setAttempt((n) => n + 1), [])
  return { check, error, state, retry }
}
