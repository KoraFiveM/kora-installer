import { api } from '@/api/client'

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms))

/**
 * Ask the panel to restart, then wait until it answers again: either with a
 * different version, or at all after it has been down at least once.
 * Resolves true when it is back, false after `timeoutMs`.
 */
export async function restartPanel(timeoutMs = 60_000, intervalMs = 1_000, force = false): Promise<boolean> {
  let before: string | null = null
  try { before = (await api.meta()).version } catch { /* checked below */ }
  await api.panel.restart(force)
  let wentDown = false
  for (let waited = 0; waited < timeoutMs; waited += intervalMs) {
    await sleep(intervalMs)
    try {
      const m = await api.meta()
      if (wentDown || (before !== null && m.version !== before)) return true
    } catch {
      wentDown = true
    }
  }
  return false
}
