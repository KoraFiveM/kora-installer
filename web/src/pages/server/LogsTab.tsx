import { useEffect, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { RefreshCw } from 'lucide-react'
import { api } from '@/api/client'
import type { ServerDetail } from '@/api/types'
import { Button } from '@/components/ui/button'
import { Select } from '@/components/ui/input'
import { CopyButton } from '@/components/CopyButton'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { formatBytes } from '@/lib/utils'
import { parseAnsi } from '@/lib/ansi'

export function LogsTab({ server: s }: { server: ServerDetail }) {
  const files = useQuery({ queryKey: ['logs', s.name], queryFn: () => api.logs.list(s.name) })
  const [file, setFile] = useState('')
  useEffect(() => {
    if (!file && files.data?.length) setFile(files.data[0].name)
  }, [files.data, file])
  const tail = useQuery({ queryKey: ['log', s.name, file], queryFn: () => api.logs.tail(s.name, file, 300), enabled: !!file })
  const text = (tail.data?.lines ?? []).join('\n')
  return (
    <div className="space-y-3">
      <ErrorNotice error={files.error} />
      <div className="flex flex-wrap items-end gap-3">
        <div className="w-64">
          <Field label="Log file">
            <Select value={file} onChange={(e) => setFile(e.target.value)}>
              {(files.data ?? []).map((f) => <option key={f.name} value={f.name}>{f.name} ({formatBytes(f.size)})</option>)}
            </Select>
          </Field>
        </div>
        <Button onClick={() => void tail.refetch()} disabled={!file || tail.isFetching}><RefreshCw className={tail.isFetching ? 'animate-spin' : ''} /> Reload</Button>
        <CopyButton text={text} label="Copy log" size="default" />
      </div>
      {files.isSuccess && files.data.length === 0 && <p className="text-sm text-muted-foreground">There are no log files yet. They appear after the server has run.</p>}
      <ErrorNotice error={tail.error} />
      {file && <pre tabIndex={0} aria-label={`Last lines of ${file}`} className="h-96 overflow-auto whitespace-pre-wrap break-words rounded-lg border border-border bg-ink p-3 font-mono text-xs leading-relaxed">{text ? parseAnsi(text) : (tail.isLoading ? 'Loading...' : 'This file is empty.')}</pre>}
      <p className="text-xs text-muted-foreground">Shows the last 300 lines.</p>
    </div>
  )
}
