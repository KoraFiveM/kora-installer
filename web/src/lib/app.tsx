import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from 'react'

export interface AppAlert {
  id: number
  tone: 'info' | 'success' | 'warning' | 'danger'
  text: string
}

interface AlertsApi {
  alerts: AppAlert[]
  push: (tone: AppAlert['tone'], text: string) => void
  dismiss: (id: number) => void
}

const Ctx = createContext<AlertsApi | null>(null)
let seq = 1

/** Global banner area: any screen can call `useAlerts().push(...)`. */
export function AlertsProvider({ children }: { children: ReactNode }) {
  const [alerts, setAlerts] = useState<AppAlert[]>([])
  const dismiss = useCallback((id: number) => setAlerts((a) => a.filter((x) => x.id !== id)), [])
  const push = useCallback((tone: AppAlert['tone'], text: string) => setAlerts((a) => [...a.slice(-3), { id: seq++, tone, text }]), [])
  const value = useMemo(() => ({ alerts, push, dismiss }), [alerts, push, dismiss])
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>
}

export function useAlerts(): AlertsApi {
  const v = useContext(Ctx)
  if (!v) throw new Error('useAlerts must be used inside AlertsProvider')
  return v
}
