import { useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { ControlAction } from '@/api/types'
import { useAlerts } from '@/lib/app'
import { toNotice } from '@/components/ErrorNotice'
import { useJob } from './useJob'
import { t } from '@/i18n'
import '@/i18n/en/core'

const ING: Record<ControlAction, string> = { start: 'Starting', stop: 'Stopping', restart: 'Restarting' }
const DONE: Record<ControlAction, string> = { start: 'started', stop: 'stopped', restart: 'restarted' }

/** Start / stop / restart one server and follow the job. `pending` is the label to show while it runs. */
export function useServerControl(name: string) {
  const qc = useQueryClient()
  const { push } = useAlerts()
  const [job, setJob] = useState<string | null>(null)
  const [action, setAction] = useState<ControlAction | null>(null)
  const [starting, setStarting] = useState(false)
  const view = useJob(job, (j) => {
    const a = action
    setJob(null)
    setAction(null)
    void qc.invalidateQueries({ queryKey: ['servers'] })
    void qc.invalidateQueries({ queryKey: ['server', name] })
    if (j.state === 'failed') push('danger', `${name}: ${j.error?.message ?? t('core.useServerControl.actionDidFinish')}`)
    else if (a) push('success', `${name} ${DONE[a]}.`)
  })
  const run = async (a: ControlAction) => {
    setStarting(true)
    setAction(a)
    try {
      setJob((await api.servers.control(name, a)).job)
    } catch (e) {
      setAction(null)
      push('danger', `${name}: ${toNotice(e)?.message ?? t('core.useServerControl.actionDidStart')}`)
    } finally {
      setStarting(false)
    }
  }
  const busy = starting || view.phase === 'running'
  return { run, busy, pending: busy && action ? ING[action] : undefined }
}
