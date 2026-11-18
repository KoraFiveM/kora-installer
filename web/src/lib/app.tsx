import { notifyIfAway } from '@/lib/shell'
import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from 'react'
import { t } from '@/i18n'
import '@/i18n/en/core/app'

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
  const push = useCallback((tone: AppAlert['tone'], text: string) => {
    setAlerts((a) => [...a.slice(-3), { id: seq++, tone, text }])
    // Inside the desktop app and not looking at it: also a desktop notification (a no-op in a browser).
    if (tone === 'success' || tone === 'danger' || tone === 'warning') notifyIfAway({ title: tone === 'success' ? 'Kora' : t('core.app.koraNeedsLook'), body: text, urgent: tone === 'danger' })
  }, [])
  const value = useMemo(() => ({ alerts, push, dismiss }), [alerts, push, dismiss])
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>
}

export function useAlerts(): AlertsApi {
  const v = useContext(Ctx)
  if (!v) throw new Error('useAlerts must be used inside AlertsProvider') // i18n-ignore: a developer error
  return v
}
