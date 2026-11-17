import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { ExternalLink, FolderOpen } from 'lucide-react'
import { api } from '@/api/client'
import { Button } from '@/components/ui/button'
import { CopyButton } from '@/components/CopyButton'
import { toNotice } from '@/components/ErrorNotice'
import { useRole } from '@/hooks/useRole'
import { txAdminLink, type TxAdminLink } from '@/lib/serverLinks'

type Size = 'sm' | 'default'

/**
 * Is the browser on the same computer as Kora, with a desktop in front of it? Only then can a click open a
 * window the person will see. Remote (Tailscale, token), a tunnel to a headless machine and "not known yet"
 * all count as no, so they get "Copy path".
 */
export function useCanOpenFolder(): boolean {
  const sys = useQuery({ queryKey: ['system'], queryFn: api.system })
  const { remote } = useRole()
  return !remote && sys.data?.local === true
}

/** "Open folder" on the computer that runs Kora; "Copy path" from anywhere else (see [`useCanOpenFolder`]). */
export function OpenFolderButton({ name, dir, size = 'sm' }: { name: string; dir: string; size?: Size }) {
  const canOpen = useCanOpenFolder()
  const [failed, setFailed] = useState<string | null>(null)
  if (!canOpen || failed) {
    return (
      <span className="inline-flex flex-wrap items-center gap-2">
        <CopyButton text={dir} label="Copy path" size={size} />
        {failed && <span className="text-xs text-muted-foreground">Could not open it here: {failed}</span>}
      </span>
    )
  }
  return (
    <Button size={size} onClick={async () => { try { await api.servers.openFolder(name) } catch (e) { setFailed(toNotice(e)?.message ?? 'unknown error') } }}>
      <FolderOpen /> Open folder
    </Button>
  )
}

/** The txAdmin address for this browser, and what to say about it. */
export function useTxAdminLink(port: number): TxAdminLink {
  const sys = useQuery({ queryKey: ['system'], queryFn: api.system })
  return txAdminLink(window.location.hostname, port, sys.data?.local === true)
}

/** A real button that opens txAdmin in a new tab. Greyed out (with the reason as its tooltip) while txAdmin is not answering. */
export function TxAdminButton({ port, up, size = 'sm' }: { port: number; up: boolean; size?: Size }) {
  const link = useTxAdminLink(port)
  if (!up) return <Button size={size} variant="default" disabled title="txAdmin answers once the server is running."><ExternalLink /> Open txAdmin</Button>
  return (
    <Button asChild size={size} variant="default">
      <a href={link.url} target="_blank" rel="noopener noreferrer" title={link.hint ?? undefined}><ExternalLink /> Open txAdmin</a>
    </Button>
  )
}

/** Copies the address players on the network type to join. Nothing when Kora could not work it out. */
export function CopyConnectButton({ connect, size = 'sm' }: { connect: string | null | undefined; size?: Size }) {
  if (!connect) return null
  return <CopyButton text={connect} label="Copy connect address" size={size} />
}
