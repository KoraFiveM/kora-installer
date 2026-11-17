import { useEffect, useRef, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { ArrowDown, Play } from 'lucide-react'
import { api } from '@/api/client'
import { useRole, whyNot } from '@/hooks/useRole'
import { useServerControl } from '@/hooks/useServerControl'
import type { ServerDetail } from '@/api/types'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { ErrorNotice } from '@/components/ErrorNotice'
import { parseAnsi } from '@/lib/ansi'
import { t } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/servers'

const MAX_LINES = 1000

function lineOf(data: string): string {
  try {
    const j = JSON.parse(data) as unknown
    if (typeof j === 'string') return j
    if (j && typeof j === 'object' && 'line' in j) return String((j as { line: unknown }).line)
  } catch {
    /* plain text */
  }
  return data
}

export function ConsoleTab({ server: s }: { server: ServerDetail }) {
  const qc = useQueryClient()
  const [lines, setLines] = useState<string[]>([])
  const [paused, setPaused] = useState(false)
  const [cmd, setCmd] = useState('')
  const [err, setErr] = useState<unknown>(null)
  const [enabling, setEnabling] = useState(false)
  const history = useRef<string[]>([])
  const hpos = useRef(-1)
  const box = useRef<HTMLDivElement>(null)
  const stick = useRef(true)
  const role = useRole()
  const readOk = role.can('operator')
  const ctl = useServerControl(s.name)
  const startOff = whyNot(role, 'operator')

  useEffect(() => {
    if (!s.console_enabled || !readOk || typeof EventSource === 'undefined') return
    const es = new EventSource(api.console.streamUrl(s.name))
    const add = (m: Event) => setLines((l) => [...l, lineOf((m as MessageEvent<string>).data)].slice(-MAX_LINES))
    es.addEventListener('line', add)
    es.onmessage = add
    return () => es.close()
  }, [s.name, s.console_enabled, readOk])

  useEffect(() => {
    const el = box.current
    if (el && stick.current) el.scrollTop = el.scrollHeight
  }, [lines])

  const onScroll = () => {
    const el = box.current
    if (!el) return
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 24
    stick.current = atBottom
    setPaused(!atBottom)
  }
  const jump = () => {
    const el = box.current
    if (el) el.scrollTop = el.scrollHeight
    stick.current = true
    setPaused(false)
  }

  if (!readOk) return <Alert tone="info" title={t('servers.consoleTab.consoleAvailable')}>{whyNot(role, 'operator')}</Alert>

  if (!s.console_enabled) {
    return (
      <div className="max-w-xl space-y-4">
        <Alert tone="info" title={t('servers.consoleTab.consoleSwitched')}>{t('servers.consoleTab.showLiveServerOutput')}</Alert>
        <Button variant="default" disabled={enabling || !role.can('admin')} title={whyNot(role, 'admin') ?? undefined} onClick={async () => { setEnabling(true); setErr(null); try { await api.console.enable(s.name); await qc.invalidateQueries({ queryKey: ['server', s.name] }) } catch (e) { setErr(e) } finally { setEnabling(false) } }}>{t('servers.consoleTab.enableConsole')}</Button>
        <ErrorNotice error={err} />
      </div>
    )
  }

  const send = async () => {
    const c = cmd.trim()
    if (!c) return
    history.current.push(c)
    hpos.current = -1
    setCmd('')
    setErr(null)
    try {
      await api.console.send(s.name, c)
    } catch (e) {
      setErr(e)
    }
  }

  return (
    <div className="space-y-3">
      {s.status !== 'running' && (
        <Alert tone="warning">
          <p>{t('servers.consoleTab.serverRunningThereNo')}</p>
          {s.status === 'stopped' && (
            <Button size="sm" className="mt-2" disabled={ctl.busy || startOff !== null} title={startOff ?? undefined} onClick={() => void ctl.run('start')}>
              <Play /> {ctl.pending ?? t('servers.consoleTab.startServer')}
            </Button>
          )}
        </Alert>
      )}
      <div className="relative">
        <div ref={box} onScroll={onScroll} role="log" aria-label={t('servers.consoleTab.serverConsoleOutput')} tabIndex={0} className="h-96 overflow-y-auto rounded-lg border border-border bg-ink p-3 font-mono text-xs leading-relaxed">
          {lines.length === 0 ? <span className="text-muted-foreground">{t('servers.consoleTab.waitingOutput')}</span> : lines.map((l, i) => <div key={i} className="whitespace-pre-wrap break-words">{parseAnsi(l)}</div>)}
        </div>
        {paused && <Button size="sm" className="absolute bottom-3 right-5" onClick={jump}>{rich('servers.consoleTab.arrowdownNodePausedJump', { arrowDown: <ArrowDown /> })}</Button>}
      </div>
      <form className="flex gap-2" onSubmit={(e) => { e.preventDefault(); void send() }}>
        <label htmlFor="console-cmd" className="sr-only">{t('servers.consoleTab.consoleCommand')}</label>
        <Input
          id="console-cmd"
          className="font-mono"
          value={cmd}
          placeholder={t('servers.consoleTab.typeCommandExampleStatus')}
          autoComplete="off"
          disabled={!role.canDestroy}
          onChange={(e) => setCmd(e.target.value)}
          onKeyDown={(e) => {
            const h = history.current
            if (e.key === 'ArrowUp' && h.length) {
              e.preventDefault()
              hpos.current = hpos.current < 0 ? h.length - 1 : Math.max(0, hpos.current - 1)
              setCmd(h[hpos.current])
            } else if (e.key === 'ArrowDown' && hpos.current >= 0) {
              e.preventDefault()
              hpos.current += 1
              if (hpos.current >= h.length) { hpos.current = -1; setCmd('') } else setCmd(h[hpos.current])
            }
          }}
        />
        <Button type="submit" variant="default" disabled={!cmd.trim() || !role.canDestroy}>{t('servers.consoleTab.send')}</Button>
      </form>
      {!role.canDestroy && <p className="text-xs text-muted-foreground">{whyNot(role, 'admin', true)}</p>}
      <ErrorNotice error={err} />
    </div>
  )
}
